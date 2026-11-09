use crate::mvcc::{Fate, Writers};

use super::chain;
use super::page::{self, Arrangement, Slot};
use super::version::{Flags, Header};

/// What pruning makes of a page.
#[derive(Debug, Default)]
pub(super) struct Plan {
    /// Each slot to empty, with what it is left holding: nothing, a dead mark or a redirect.
    pub release: Vec<(usize, Slot)>,
    /// Each version whose deleter aborted.
    pub clear: Vec<usize>,
    /// Each slot named by index entries whose chain on the page holds no live version, so its
    /// entries must go before it can be freed.
    pub dead: Vec<usize>,
    /// Every version left is visible to every snapshot, and no slot is left dead.
    pub all_visible: bool,
}

/// Returns the fate of the version in each slot of `page`, `None` for a slot holding none, and
/// whether its deleter is to be cleared first. A version whose deleter is cleared is judged again
/// as it is left.
///
/// Panics if `judge` clears a version with no deleter.
pub(super) fn judge<E>(
    page: &[u8],
    mut judge: impl FnMut(&Writers) -> Result<Fate, E>,
) -> Result<Vec<Option<(Fate, bool)>>, E> {
    let mut out = Vec::new();
    for s in 0..page::slot_count(page) {
        let Slot::Normal { at, .. } = page::get(page, s) else {
            out.push(None);
            continue;
        };
        let mut header = Header::read(&page[at]);
        let f = judge(&header.writers)?;
        if f != Fate::Clear {
            out.push(Some((f, false)));
            continue;
        }
        header.writers = header.writers.cleared();
        header.next = None;
        let again = judge(&header.writers)?;
        assert_ne!(
            Fate::Clear,
            again,
            "a version with no deleter has its deleter cleared"
        );
        out.push(Some((again, true)));
    }
    Ok(out)
}

/// Returns how to prune `page`, page `pid` of its heap, its versions' fates being `fates` as
/// [`judge`] returns them. A slot `cleaned` accepts has lost its index entries and is freed.
///
/// A chain of heap-only versions starts at a root, as `chain::roots` finds it. On a slotted page,
/// a chain whose leading versions are dead keeps its start as a redirect to its first live version
/// and frees the others; a chain with no live version leaves its start dead. On a fixed-slot page
/// a chain is freed only whole, once its entries are gone.
///
/// Panics if a slot `cleaned` accepts still leads to a live version.
pub(super) fn plan(
    page: &[u8],
    pid: u64,
    fates: &[Option<(Fate, bool)>],
    cleaned: &dyn Fn(usize) -> bool,
) -> Plan {
    let n = page::slot_count(page);
    let slots: Vec<Slot> = (0..n).map(|s| page::get(page, s)).collect();
    let fate: Vec<Option<Fate>> = fates.iter().map(|f| f.map(|(f, _)| f)).collect();
    let cleared: Vec<bool> = fates.iter().map(|f| f.is_some_and(|(_, c)| c)).collect();
    let links: Vec<Option<usize>> = (0..n)
        .map(|s| chain::link(page, pid, s).filter(|_| !cleared[s]))
        .collect();
    let fixed = matches!(page::arrangement(page), Arrangement::Fixed { .. });
    let mut out: Vec<Option<Slot>> = vec![None; n];
    let mut chained = vec![false; n];
    let mut plan = Plan::default();
    for (e, slot) in slots.iter().enumerate() {
        if *slot == Slot::Dead {
            plan.dead.push(e);
            if cleaned(e) {
                out[e] = Some(Slot::Unused);
            }
        }
    }
    for (e, members) in chain::roots(page, |s| links[s]) {
        for &m in &members {
            chained[m] = true;
        }
        let live = members.iter().position(|&m| fate[m] != Some(Fate::Remove));
        match live {
            Some(k) => {
                assert!(
                    !cleaned(e),
                    "slot {e} lost its index entries but leads to a live version"
                );
                if k > 0 && !fixed {
                    out[e] = Some(Slot::Redirect(members[k] as u16));
                    for &m in members[..k].iter().filter(|&&m| m != e) {
                        out[m] = Some(Slot::Unused);
                    }
                }
            }
            None => {
                plan.dead.push(e);
                if cleaned(e) || !fixed {
                    out[e] = Some(if cleaned(e) { Slot::Unused } else { Slot::Dead });
                    for &m in members.iter().filter(|&&m| m != e) {
                        out[m] = Some(Slot::Unused);
                    }
                }
            }
        }
    }
    plan.dead.sort_unstable();
    for (s, slot) in slots.iter().enumerate() {
        if let Slot::Normal { flags, .. } = slot
            && flags.contains(Flags::HEAP_ONLY)
            && !chained[s]
            && fate[s] == Some(Fate::Remove)
        {
            out[s] = Some(Slot::Unused);
        }
    }
    plan.all_visible = true;
    for (s, slot) in slots.iter().enumerate() {
        match (&out[s], slot) {
            (Some(content), _) => {
                plan.all_visible &= *content != Slot::Dead;
                if content != slot {
                    plan.release.push((s, content.clone()));
                }
            }
            (None, Slot::Dead) => plan.all_visible = false,
            (None, Slot::Normal { .. }) => {
                plan.all_visible &= fate[s] == Some(Fate::AllVisible);
                if cleared[s] {
                    plan.clear.push(s);
                }
            }
            (None, _) => {}
        }
    }
    plan
}

pub(super) fn apply(page: &mut [u8], plan: &Plan) {
    for &s in &plan.clear {
        let Slot::Normal { flags, at } = page::get(page, s) else {
            unreachable!("a cleared slot holds a version")
        };
        let mut header = Header::read(&page[at.clone()]);
        header.writers = header.writers.cleared();
        header.next = None;
        header.write(&mut page[at]);
        page::set_flags(
            page,
            s,
            flags.without(Flags::MOVED).without(Flags::HOT_UPDATED),
        );
    }
    for (s, content) in &plan.release {
        page::release(page, *s, content);
    }
    page::compact(page);
}
