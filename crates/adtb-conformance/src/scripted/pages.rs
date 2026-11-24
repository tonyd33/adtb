//! The page edits of `scribble`, and the probes of the holding `verify` and of a heap's pages.

use std::collections::{BTreeMap, BTreeSet};

use adtb::key::Tid;
use adtb::testing::{Edit, Flags, Header, Laid, PageKind, Pid, Plant, Slot, Testing, Xid};
use adtb_lang::textual::print;
use adtb_sim::SimRuntime;

use super::fault::Page;
use super::role::collected;

/// The names of the flags of a version, in bit order.
const FLAGS: [(Flags, &str); 5] = [
    (Flags::MOVED, "MOVED"),
    (Flags::RETAGGED_IN, "RETAGGED_IN"),
    (Flags::HOT_UPDATED, "HOT_UPDATED"),
    (Flags::HEAP_ONLY, "HEAP_ONLY"),
    (Flags::KEPT, "KEPT"),
];

/// Make the edit `body` states to page `pid`, which `page` names: on a heap, directory or store
/// page `kind KIND`, `entry N names PAGE`, `no map`, or `slot N` followed by `broken`,
/// `deleter unreserved`, `inserter unreserved`, `writer unreserved` or `count C`; on an index `entry N again at slot S`, entry N of the
/// index added again naming slot S of its page. Returns the page the edit names besides, with
/// its name.
pub(super) fn scribble(
    t: Testing<'_, SimRuntime>,
    page: &Page,
    pid: Pid,
    body: &str,
) -> Result<Option<(Pid, String)>, String> {
    let text = body.trim();
    let bad = || format!("`{text}` is no edit");
    let words: Vec<&str> = text.split(' ').collect();
    let mut named = None;
    let edit = match words[..] {
        ["entry", at, "again", "at", "slot", slot] => {
            let at: usize = at.parse().map_err(|_| bad())?;
            let slot = slot.parse().map_err(|_| bad())?;
            return again(t, page, at, slot).map(Some);
        }
        ["kind", kind] => Edit::Kind(kind_named(kind).ok_or_else(bad)?),
        ["no", "map"] => Edit::NoMap,
        ["entry", at, "names", ..] => {
            let other = text.splitn(4, ' ').nth(3).ok_or_else(bad)?;
            let other = Page::parse(other).ok_or_else(bad)?;
            let names = other.pid(t)?;
            named = Some((names, other.to_string()));
            Edit::Entry {
                at: at.parse().map_err(|_| bad())?,
                names,
            }
        }
        ["slot", slot, ref rest @ ..] => {
            let slot = slot.parse().map_err(|_| bad())?;
            match rest {
                ["broken"] => Edit::Broken(slot),
                ["deleter", "unreserved"] => Edit::Deleter {
                    slot,
                    by: t.unreserved(),
                },
                ["inserter", "unreserved"] => Edit::Inserter {
                    slot,
                    by: t.unreserved(),
                },
                ["writer", "unreserved"] => Edit::Writer {
                    slot,
                    by: t.unreserved(),
                },
                ["count", count] => Edit::Count {
                    slot,
                    count: count.parse().map_err(|_| bad())?,
                },
                _ => return Err(bad()),
            }
        }
        _ => return Err(bad()),
    };
    t.scribble(pid, edit)
        .map_err(|e| format!("the edit is not made: {e:?}"))?;
    Ok(named)
}

/// Add entry `at` of the index `page` names again, naming slot `slot` of the page its TID names.
/// Returns that page, with its name.
fn again(
    t: Testing<'_, SimRuntime>,
    page: &Page,
    at: usize,
    slot: u16,
) -> Result<(Pid, String), String> {
    let Page::Index { relation, clause } = page else {
        return Err(format!("`{page}` names no index"));
    };
    let index = t.index(relation, clause).ok_or("no such index")?;
    let entries = t.all_entries(index).map_err(|e| format!("{e:?}"))?;
    let mut entry = entries
        .get(at)
        .cloned()
        .ok_or("the index has no such entry")?;
    let held = entry.tid.page();
    entry.tid = Tid::new(held, slot).ok_or("no such slot")?;
    t.plant(Plant::AddEntry { index, entry })
        .map_err(|e| format!("the entry is not added: {e:?}"))?;
    Ok((Pid(held), format!("the page entry {at} of {page} names")))
}

fn kind_named(name: &str) -> Option<PageKind> {
    (1..=u16::MAX)
        .map_while(PageKind::from_u16)
        .find(|k| format!("{k:?}") == name)
}

/// Returns the name `roles` gives `pid`.
pub(super) fn role(roles: &BTreeMap<Pid, String>, pid: Pid) -> String {
    roles
        .get(&pid)
        .cloned()
        .unwrap_or_else(|| "a page no fault names".to_owned())
}

/// Returns each row of the run of `verify` that holds the database, sorted, with each page
/// number it names written as the name `roles` gives it.
pub(super) fn held(t: Testing<'_, SimRuntime>, roles: &BTreeMap<Pid, String>) -> Vec<String> {
    let (form, rows) = match t.verify_holding().and_then(collected) {
        Ok(found) => found,
        Err(e) => return vec![format!("verify fails: {e:?}")],
    };
    let mut out: Vec<String> = rows
        .iter()
        .map(|r| named(&print(r, &form).unwrap_or_else(|e| e.message), roles))
        .collect();
    out.sort();
    out
}

/// Returns `row` with each `page N` written as `roles` names page N.
fn named(row: &str, roles: &BTreeMap<Pid, String>) -> String {
    let mut out = String::new();
    let mut rest = row;
    while let Some(at) = rest.find("page ") {
        let (before, after) = rest.split_at(at + "page ".len());
        let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        match after[..digits].parse() {
            Ok(n) => {
                out.push_str(&before[..at]);
                out.push_str(&role(roles, Pid(n)));
            }
            Err(_) => out.push_str(before),
        }
        rest = &after[digits..];
    }
    out.push_str(rest);
    out
}

/// Returns the layout of the heap of `ctor` of `relation`: a line for each position of its
/// directory with the number of slots of the page there, then, unless `pages`, one for each of
/// those slots. A transaction id is `xN`, by rank among the ids the layout names; a TID is
/// `POSITION.SLOT`, `elsewhere` off the heap's pages.
pub(super) fn laid(
    t: Testing<'_, SimRuntime>,
    relation: &str,
    ctor: &str,
    pages: bool,
) -> Vec<String> {
    let Some(heap) = t.heap(relation, ctor) else {
        return vec!["no such constructor".to_owned()];
    };
    let laid = match t.layout(heap) {
        Ok(laid) => laid,
        Err(e) => return vec![format!("the layout is not read: {e:?}")],
    };
    let mut positions = BTreeMap::new();
    let mut xids = BTreeSet::new();
    for (i, l) in laid.iter().enumerate() {
        if let Laid::Page { pid, slots, .. } = l {
            positions.insert(*pid, i);
            for h in slots.iter().filter_map(|(_, h)| h.as_ref()) {
                xids.insert(h.writers.inserter());
                xids.extend(h.writers.deleter());
            }
        }
    }
    let ranks: BTreeMap<Xid, usize> = xids.into_iter().zip(0..).collect();
    let mut out = Vec::new();
    for (i, l) in laid.iter().enumerate() {
        let Laid::Page {
            kind,
            format,
            flagged,
            mapped,
            slots,
            ..
        } = l
        else {
            out.push(format!("{i}: hole"));
            continue;
        };
        let kind = kind.map_or_else(|| "no kind".to_owned(), |k| format!("{k:?}"));
        let mut line = format!("{i}: {kind} format {format}, {} slots", slots.len());
        if *flagged {
            line.push_str(", flagged all visible");
        }
        if *mapped {
            line.push_str(", mapped all visible");
        }
        out.push(line);
        if pages {
            continue;
        }
        let tid = |page: u64, slot: u16| {
            positions
                .get(&Pid(page))
                .map_or_else(|| "elsewhere".to_owned(), |p| format!("{p}.{slot}"))
        };
        for (s, (slot, header)) in slots.iter().enumerate() {
            let shown = match (slot, header) {
                (Slot::Normal { flags, .. }, Some(h)) => normal(*flags, h, &ranks, &tid),
                (Slot::Redirect(to), _) => format!("Redirect {i}.{to}"),
                (Slot::Normal { .. }, None) => "Normal".to_owned(),
                (Slot::Unused, _) => "Unused".to_owned(),
                (Slot::Dead, _) => "Dead".to_owned(),
            };
            out.push(format!("{i}.{s}: {shown}"));
        }
    }
    out
}

fn normal(
    flags: Flags,
    h: &Header,
    ranks: &BTreeMap<Xid, usize>,
    tid: &dyn Fn(u64, u16) -> String,
) -> String {
    let names: Vec<&str> = FLAGS
        .iter()
        .filter(|(f, _)| flags.contains(*f))
        .map(|(_, n)| *n)
        .collect();
    let mut out = format!(
        "Normal {}, x{}",
        if names.is_empty() {
            "NONE".to_owned()
        } else {
            names.join("|")
        },
        ranks[&h.writers.inserter()]
    );
    if let Some(d) = h.writers.deleter() {
        out.push_str(&format!(" to x{}", ranks[&d]));
    }
    if let Some(next) = h.next {
        out.push_str(&format!(", next {}", tid(next.page(), next.slot())));
    }
    out
}
