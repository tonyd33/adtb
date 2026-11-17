use super::page::{CONTENTS, Slot, align8, u16_at, u32_at};
use super::version::{Flags, HEADER_LEN};

const UPPER: usize = 28;
const COUNT: usize = 32;
const POINTERS: usize = 40;
const POINTER: usize = 8;

const UNUSED: u8 = 0;
const NORMAL: u8 = 1;
const DEAD: u8 = 2;
const REDIRECT: u8 = 3;

/// Returns the length of the longest version a page of `page_size` bytes holds.
pub fn max_version(page_size: usize) -> usize {
    (page_size - POINTERS - POINTER) & !7
}

pub fn count(page: &[u8]) -> usize {
    usize::from(u16_at(page, COUNT))
}

fn upper(page: &[u8]) -> usize {
    u32_at(page, UPPER) as usize
}

fn set_upper(page: &mut [u8], upper: usize) {
    page[UPPER..UPPER + 4].copy_from_slice(&(upper as u32).to_le_bytes());
}

fn set_count(page: &mut [u8], n: usize) {
    page[COUNT..COUNT + 2].copy_from_slice(&(n as u16).to_le_bytes());
}

/// Format `page` as an empty slotted page.
pub fn init(page: &mut [u8]) {
    page[CONTENTS..].fill(0);
    set_upper(page, page.len());
}

struct Pointer {
    offset: usize,
    len: usize,
    state: u8,
    flags: u16,
}

impl Pointer {
    fn of(b: [u8; POINTER]) -> Pointer {
        Pointer {
            offset: usize::from(u16::from_le_bytes([b[0], b[1]])),
            len: usize::from(u16::from_le_bytes([b[2], b[3]])),
            state: b[4],
            flags: u16::from_le_bytes([b[6], b[7]]),
        }
    }
}

fn pointer(page: &[u8], slot: usize) -> Pointer {
    let at = POINTERS + POINTER * slot;
    let mut b = [0; POINTER];
    b.copy_from_slice(&page[at..at + POINTER]);
    Pointer::of(b)
}

fn set_pointer(page: &mut [u8], slot: usize, p: &Pointer) {
    let at = POINTERS + POINTER * slot;
    page[at..at + 2].copy_from_slice(&(p.offset as u16).to_le_bytes());
    page[at + 2..at + 4].copy_from_slice(&(p.len as u16).to_le_bytes());
    page[at + 4] = p.state;
    page[at + 5] = 0;
    page[at + 6..at + 8].copy_from_slice(&p.flags.to_le_bytes());
}

pub fn get(page: &[u8], slot: usize) -> Slot {
    if slot >= count(page) {
        return Slot::Unused;
    }
    let p = pointer(page, slot);
    match p.state {
        UNUSED => Slot::Unused,
        NORMAL => Slot::Normal {
            flags: Flags(p.flags),
            at: p.offset..p.offset + p.len,
        },
        DEAD => Slot::Dead,
        REDIRECT => Slot::Redirect(p.offset as u16),
        s => unreachable!("line pointer {slot} of a framed page has state {s}"),
    }
}

pub fn set_flags(page: &mut [u8], slot: usize, flags: Flags) {
    let mut p = pointer(page, slot);
    assert_eq!(NORMAL, p.state, "flags of a slot holding no version");
    p.flags = flags.0;
    set_pointer(page, slot, &p);
}

/// Returns the bytes not taken by line pointers or tuples, wherever they lie.
fn unclaimed(page: &[u8]) -> usize {
    let n = count(page);
    let tuples: usize = (0..n)
        .map(|s| pointer(page, s))
        .filter(|p| p.state == NORMAL)
        .map(|p| align8(p.len))
        .sum();
    page.len() - POINTERS - POINTER * n - tuples
}

/// Returns the bytes free for new versions, after a line pointer for one more.
pub fn free(page: &[u8]) -> usize {
    unclaimed(page).saturating_sub(POINTER)
}

/// Move every tuple to the end of the page, leaving the free space in one run. TIDs do not
/// change.
pub fn compact(page: &mut [u8]) {
    let n = count(page);
    let tuples: Vec<(usize, Vec<u8>)> = (0..n)
        .filter_map(|s| {
            let p = pointer(page, s);
            (p.state == NORMAL).then(|| (s, page[p.offset..p.offset + p.len].to_vec()))
        })
        .collect();
    let mut top = page.len();
    for (s, bytes) in tuples {
        top -= align8(bytes.len());
        page[top..top + bytes.len()].copy_from_slice(&bytes);
        let mut p = pointer(page, s);
        p.offset = top;
        set_pointer(page, s, &p);
    }
    page[POINTERS + POINTER * n..top].fill(0);
    set_upper(page, top);
}

pub enum Content<'a> {
    Version(&'a [u8], Flags),
    Dead,
    Redirect(u16),
}

/// Fill slot `slot` with `content`, adding unused line pointers up to it. Returns `false`,
/// changing nothing, when the slot is taken or the page lacks the room.
pub fn put(page: &mut [u8], slot: usize, content: Content) -> bool {
    let n = count(page);
    if slot < n && pointer(page, slot).state != UNUSED {
        return false;
    }
    let pointers = (slot + 1).saturating_sub(n);
    let need = match content {
        Content::Version(v, _) => align8(v.len()),
        Content::Dead | Content::Redirect(_) => 0,
    };
    if unclaimed(page) < need + POINTER * pointers {
        return false;
    }
    let lower = POINTERS + POINTER * n.max(slot + 1);
    if upper(page) < lower + need {
        compact(page);
    }
    if pointers > 0 {
        set_count(page, slot + 1);
    }
    let p = match content {
        Content::Version(v, flags) => {
            let top = upper(page) - need;
            page[top..top + v.len()].copy_from_slice(v);
            page[top + v.len()..top + need].fill(0);
            set_upper(page, top);
            Pointer {
                offset: top,
                len: v.len(),
                state: NORMAL,
                flags: flags.0,
            }
        }
        Content::Dead => Pointer {
            offset: 0,
            len: 0,
            state: DEAD,
            flags: 0,
        },
        Content::Redirect(to) => Pointer {
            offset: usize::from(to),
            len: 0,
            state: REDIRECT,
            flags: 0,
        },
    };
    set_pointer(page, slot, &p);
    true
}

/// Empty slot `slot` of whatever it holds, leaving it `content`, and drop the line pointers of
/// unused slots at the end of the page.
pub fn release(page: &mut [u8], slot: usize, content: Option<Content>) {
    let p = match content {
        None => Pointer {
            offset: 0,
            len: 0,
            state: UNUSED,
            flags: 0,
        },
        Some(Content::Dead) => Pointer {
            offset: 0,
            len: 0,
            state: DEAD,
            flags: 0,
        },
        Some(Content::Redirect(to)) => Pointer {
            offset: usize::from(to),
            len: 0,
            state: REDIRECT,
            flags: 0,
        },
        Some(Content::Version(..)) => panic!("release leaves no version in a slot"),
    };
    set_pointer(page, slot, &p);
    let mut n = count(page);
    while n > 0 && pointer(page, n - 1).state == UNUSED {
        n -= 1;
        let at = POINTERS + POINTER * n;
        page[at..at + POINTER].fill(0);
    }
    set_count(page, n);
}

/// Write `version` into the first unused slot. Returns the slot, or `None` if the page lacks the
/// room.
pub fn insert(page: &mut [u8], version: &[u8], flags: Flags) -> Option<usize> {
    let n = count(page);
    let slot = (0..n)
        .find(|&s| pointer(page, s).state == UNUSED)
        .unwrap_or(n);
    if slot >= usize::from(u16::MAX) {
        return None;
    }
    put(page, slot, Content::Version(version, flags)).then_some(slot)
}

/// Returns whether the line pointers and tuples of a slotted page fit the page: every line
/// pointer is sound, every tuple at least a version header long, and the tuples together no
/// longer than the room after the line pointers.
pub fn framed(page: &[u8]) -> bool {
    let n = count(page);
    let (lower, top) = (POINTERS + POINTER * n, upper(page));
    if !(lower..=page.len()).contains(&top) {
        return false;
    }
    let mut tuples = 0;
    for b in page[POINTERS..lower].as_chunks::<POINTER>().0 {
        let p = Pointer::of(*b);
        if fault(page, &p, n, top).is_some() {
            return false;
        }
        if p.state == NORMAL {
            tuples += align8(p.len);
        }
    }
    tuples <= page.len() - lower
}

/// What is wrong with one line pointer.
enum Fault {
    State(u8),
    Bytes,
    Redirect,
}

/// Returns what is wrong with line pointer `p` of `page`, a page of `n` line pointers whose
/// tuples start at `top`.
fn fault(page: &[u8], p: &Pointer, n: usize, top: usize) -> Option<Fault> {
    match p.state {
        NORMAL => (p.offset < top
            || !p.offset.is_multiple_of(8)
            || p.len < HEADER_LEN
            || p.offset + p.len > page.len())
        .then_some(Fault::Bytes),
        REDIRECT => (p.offset >= n).then_some(Fault::Redirect),
        UNUSED | DEAD => None,
        state => Some(Fault::State(state)),
    }
}

/// Returns what is wrong with the line pointers and tuples of a slotted page.
pub fn check(page: &[u8]) -> Vec<String> {
    let n = count(page);
    let top = upper(page);
    if !(POINTERS + POINTER * n..=page.len()).contains(&top) {
        return vec![format!("has {n} line pointers and tuples from {top}")];
    }
    let mut problems = Vec::new();
    let mut tuples = Vec::new();
    for s in 0..n {
        let p = pointer(page, s);
        match fault(page, &p, n, top) {
            Some(Fault::Bytes) => problems.push(format!(
                "slot {s} holds bytes {}..{}",
                p.offset,
                p.offset + p.len
            )),
            Some(Fault::Redirect) => problems.push(format!("slot {s} redirects past the slots")),
            Some(Fault::State(state)) => problems.push(format!("slot {s} has state {state}")),
            None if p.state == NORMAL => {
                let end = p.offset + align8(p.len).min(page.len() - p.offset);
                tuples.push((p.offset, end, s));
            }
            None => {}
        }
    }
    tuples.sort_unstable();
    for w in tuples.windows(2) {
        if w[0].1 > w[1].0 {
            problems.push(format!("slots {} and {} overlap", w[0].2, w[1].2));
        }
    }
    problems
}
