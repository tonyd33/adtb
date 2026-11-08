//! The page LSN map: the last LSN of each page whose records the log no longer holds, in a radix
//! tree of `height` levels. A leaf holds one LSN per page; an interior page holds, for each
//! child, its pid and the least LSN it may have. A zero word holds nothing.

use std::collections::{BTreeMap, HashMap};

use crate::runtime::Runtime;

use super::block::{LogImage, LogSource, Mode, scan};
use super::error::Damage;
use super::page::{self, HEADER_LEN, PageKind};
use super::pager::{PageError, Pager};
use super::quarantine::Quarantine;
use super::record::{MapRoot, Record};
use super::{Lsn, Pid};

const FORMAT: u16 = 1;

/// What a page read in from the file is checked against.
#[derive(Debug)]
pub(crate) enum Gate {
    /// Recovery is replaying the log; no page has an expected LSN until it ends.
    Recovering,
    Armed(Expected),
    /// The open could not read the map or prove the retained log: the database is held
    /// quarantined and no page is checked.
    Off,
}

impl Gate {
    pub(crate) fn expected(&self, pid: Pid) -> Option<Lsn> {
        match self {
            Gate::Armed(e) => e.get(pid),
            Gate::Recovering | Gate::Off => None,
        }
    }

    pub(crate) fn raise(&mut self, pid: Pid, lsn: Lsn) {
        if let Gate::Armed(e) = self {
            e.raise(pid, lsn);
        }
    }

    /// Returns the number of pages whose expected LSN is past their folded entry.
    #[cfg(test)]
    pub(crate) fn later(&self) -> usize {
        match self {
            Gate::Armed(e) => e.logged.len(),
            Gate::Recovering | Gate::Off => 0,
        }
    }

    /// Returns one past the last page with an expected LSN, 0 when none has one.
    pub(crate) fn end(&self) -> u64 {
        match self {
            Gate::Armed(e) => e.end(),
            Gate::Recovering | Gate::Off => 0,
        }
    }
}

/// The LSN each page must have at least when read in.
#[derive(Debug, Default)]
pub(crate) struct Expected {
    /// The page LSN map's entry of each page, by pid, zero for none.
    folded: Vec<Lsn>,
    /// Each page whose last record is past its folded entry, with that record's LSN.
    logged: HashMap<Pid, Lsn>,
}

impl Expected {
    pub(crate) fn new(folded: Vec<Lsn>) -> Expected {
        Expected {
            folded,
            logged: HashMap::new(),
        }
    }

    pub(crate) fn get(&self, pid: Pid) -> Option<Lsn> {
        let folded = self.folded.get(pid.0 as usize).copied();
        self.logged.get(&pid).copied().or(folded)
    }

    fn end(&self) -> u64 {
        let folded = self
            .folded
            .iter()
            .rposition(|&l| l > Lsn(0))
            .map_or(0, |i| i + 1);
        let logged = self.logged.keys().map(|p| p.0 + 1).max().unwrap_or(0);
        (folded as u64).max(logged)
    }

    fn absorb<'a>(&mut self, last: impl Iterator<Item = (&'a Pid, &'a Lsn)>) {
        for (&pid, &lsn) in last {
            let at = pid.0 as usize;
            if self.folded.len() <= at {
                self.folded.resize(at + 1, Lsn(0));
            }
            self.folded[at] = self.folded[at].max(lsn);
            if self.logged.get(&pid).is_some_and(|&l| l <= self.folded[at]) {
                self.logged.remove(&pid);
            }
        }
    }

    /// Raise `pid`'s expected LSN to `lsn`, the LSN of its latest record.
    pub(crate) fn raise(&mut self, pid: Pid, lsn: Lsn) {
        if self.folded.get(pid.0 as usize).is_none_or(|&f| f < lsn) {
            self.logged.insert(pid, lsn);
        }
    }
}

/// A visit of a map page: its pid, bytes, level and the first page it covers.
type Visit<'v> = dyn FnMut(Pid, &[u8], u32, u64) + 'v;

fn word(page: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(
        page[HEADER_LEN + 8 * i..HEADER_LEN + 8 * i + 8]
            .try_into()
            .unwrap(),
    )
}

fn set_word(page: &mut [u8], i: usize, v: u64) {
    page[HEADER_LEN + 8 * i..HEADER_LEN + 8 * i + 8].copy_from_slice(&v.to_le_bytes());
}

impl<R: Runtime> Pager<R> {
    fn per_leaf(&self) -> u64 {
        ((self.page_size - HEADER_LEN) / 8) as u64
    }

    fn fanout(&self) -> u64 {
        ((self.page_size - HEADER_LEN) / 16) as u64
    }

    /// Returns the number of pages a map page at `level` covers, saturating.
    fn reach(&self, level: u32) -> u64 {
        self.per_leaf()
            .saturating_mul(self.fanout().saturating_pow(level - 1))
    }

    /// Returns the bytes of map page `pid`. Refuse it when it cannot be read, and also quarantine
    /// the database when it fails its checksum, is no map page, or its LSN is below `expected`, the
    /// LSN its parent gives it.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    fn map_page(&self, pid: Pid, expected: Lsn) -> Result<Vec<u8>, PageError> {
        let read = self.read(pid).map(|p| p.to_vec());
        let refused = match read {
            Err(e) if e.damage == Damage::Unreadable => return Err(e),
            Err(e) => e,
            Ok(page) if page::kind(&page) != PageKind::LsnMap as u16 => {
                self.damage(pid, Damage::Checksum)
            }
            Ok(page) if page::lsn(&page) < expected => {
                let found = page::lsn(&page);
                self.damage(pid, Damage::Stale { expected, found })
            }
            Ok(page) => return Ok(page),
        };
        if !matches!(self.meta().gate, Gate::Recovering) {
            self.quarantine(Quarantine::Database).ok();
        }
        Err(refused)
    }

    /// Check the map's root against the LSN the checkpoint or the last fold gave it, quarantining
    /// the database when it is refused.
    pub(super) fn check_map(&self) {
        let map = self.map_root();
        if let Some(m) = map {
            drop(self.map_page(m.root, m.lsn));
        }
    }

    /// Returns the LSN the map holds for `pid`, refusing a map page whose LSN is below the one
    /// its parent gives it and quarantining the database.
    pub(crate) fn mapped(&self, pid: Pid) -> Result<Option<Lsn>, PageError> {
        let Some(m) = self.map_root() else {
            return Ok(None);
        };
        if pid.0 >= self.reach(m.height) {
            return Ok(None);
        }
        let (mut at, mut expected) = (m.root, m.lsn);
        for level in (2..=m.height).rev() {
            let page = self.map_page(at, expected)?;
            let slot = (pid.0 / self.reach(level - 1) % self.fanout()) as usize;
            at = Pid(word(&page, 2 * slot));
            if at == Pid(0) {
                return Ok(None);
            }
            expected = Lsn(word(&page, 2 * slot + 1));
        }
        let lsn = word(
            &self.map_page(at, expected)?,
            (pid.0 % self.per_leaf()) as usize,
        );
        Ok((lsn != 0).then_some(Lsn(lsn)))
    }

    /// Returns every page of the map. Fail with the first map page refused.
    pub fn map_pages(&self) -> Result<Vec<Pid>, PageError> {
        let mut out = Vec::new();
        self.walk(&mut |pid, _, _, _| out.push(pid))?;
        Ok(out)
    }

    /// Returns the map's entry of every page it covers, by pid, zero for none. Fail with the
    /// first map page refused.
    pub(crate) fn map_entries(&self) -> Result<Vec<Lsn>, PageError> {
        let mut out = Vec::new();
        let per_leaf = self.per_leaf() as usize;
        self.walk(&mut |_, page, level, base| {
            if level == 1 {
                let base = base as usize;
                out.resize(out.len().max(base + per_leaf), Lsn(0));
                for i in 0..per_leaf {
                    out[base + i] = Lsn(word(page, i));
                }
            }
        })?;
        Ok(out)
    }

    fn walk(&self, visit: &mut Visit<'_>) -> Result<(), PageError> {
        let map = self.map_root();
        match map {
            Some(m) => self.subtree(m.root, m.lsn, m.height, 0, visit),
            None => Ok(()),
        }
    }

    fn subtree(
        &self,
        pid: Pid,
        expected: Lsn,
        level: u32,
        base: u64,
        visit: &mut Visit<'_>,
    ) -> Result<(), PageError> {
        let page = self.map_page(pid, expected)?;
        visit(pid, &page, level, base);
        if level == 1 {
            return Ok(());
        }
        let span = self.reach(level - 1);
        for slot in 0..self.fanout() as usize {
            let child = Pid(word(&page, 2 * slot));
            if child != Pid(0) {
                let (lsn, at) = (Lsn(word(&page, 2 * slot + 1)), base + slot as u64 * span);
                self.subtree(child, lsn, level - 1, at, visit)?;
            }
        }
        Ok(())
    }

    /// Raise the map's entry of each page of `last` to its LSN, adding levels above the root
    /// until it reaches the highest. A page at or past the file's end no longer exists, and is
    /// dropped. Returns that end.
    pub(crate) fn fold(&self, last: &BTreeMap<Pid, Lsn>) -> Result<u64, PageError> {
        let end = self.file_pages();
        let entries: Vec<(u64, Lsn)> = last
            .iter()
            .filter(|(p, _)| p.0 < end)
            .map(|(p, l)| (p.0, *l))
            .collect();
        let Some(&(top, _)) = entries.last() else {
            return Ok(end);
        };
        let map = self.map_root();
        let mut map = match map {
            Some(m) => m,
            None => self.grown(None)?,
        };
        while self.reach(map.height) <= top {
            map = self.grown(Some(map))?;
        }
        self.fold_into(map.root, map.lsn, map.height, &entries)?;
        Ok(end)
    }

    /// Raise the folded expected LSN of each page of `last` below `end` to its LSN, once a
    /// checkpoint names the map that holds them, dropping the gate's later LSN of a page when it
    /// is no longer later.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub(crate) fn absorb_folded(&self, last: &BTreeMap<Pid, Lsn>, end: u64) {
        if let Gate::Armed(e) = &mut self.meta().gate {
            e.absorb(last.range(..Pid(end)));
        }
    }

    /// Returns a new root one level above `below`, holding it as its first child, or a leaf
    /// root, made in the batch of the record naming it.
    fn grown(&self, below: Option<MapRoot>) -> Result<MapRoot, PageError> {
        let mut new = self.reserve();
        let root = new.page(FORMAT, PageKind::LsnMap, 0)?;
        let height = below.map_or(1, |b| b.height + 1);
        let tail = Record::MapRoot { root, height };
        let link = |p: &mut [&mut [u8]]| {
            if let Some(b) = below {
                set_word(p[0], 0, b.root.0);
                set_word(p[0], 1, b.lsn.0);
            }
        };
        self.update_with_tail(new, &[root], link, Some(tail))?;
        Ok(MapRoot {
            root,
            lsn: page::lsn(&self.read(root)?),
            height,
        })
    }

    /// Raise the entries of `entries`, ascending by pid, under map page `pid` at `level`, making
    /// each missing child in the batch that links it. Returns the page's LSN after.
    fn fold_into(
        &self,
        pid: Pid,
        expected: Lsn,
        level: u32,
        entries: &[(u64, Lsn)],
    ) -> Result<Lsn, PageError> {
        let page = self.map_page(pid, expected)?;
        if level == 1 {
            let per = self.per_leaf();
            self.update(&[pid], |p| {
                for &(at, lsn) in entries {
                    let i = (at % per) as usize;
                    set_word(p[0], i, word(p[0], i).max(lsn.0));
                }
            })?;
            return Ok(page::lsn(&self.read(pid)?));
        }
        let span = self.reach(level - 1);
        for group in entries.chunk_by(|a, b| a.0 / span == b.0 / span) {
            let slot = (group[0].0 / span % self.fanout()) as usize;
            let mut child = Pid(word(&page, 2 * slot));
            let below = Lsn(word(&page, 2 * slot + 1));
            if child == Pid(0) {
                let mut new = self.reserve();
                child = new.page(FORMAT, PageKind::LsnMap, 0)?;
                self.update_with(new, &[pid, child], |p| set_word(p[0], 2 * slot, child.0))?;
            }
            let lsn = self.fold_into(child, below, level - 1, group)?;
            self.update(&[pid], |p| set_word(p[0], 2 * slot + 1, lsn.0))?;
        }
        Ok(page::lsn(&self.read(pid)?))
    }
}

/// Returns the LSN of the last record of each page in the batches of `img` before `floor`, or
/// `None` when they cannot be read whole.
pub(crate) fn folded<S: LogSource>(
    img: &mut LogImage<S>,
    floor: u64,
) -> Option<BTreeMap<Pid, Lsn>> {
    let first = img.first_batch()?;
    let block = img.block;
    let mut last = BTreeMap::new();
    let mut whole = true;
    let s = scan(img, first, Mode::Verify, |batch, _| {
        if batch.start.0 >= floor {
            return;
        }
        let Ok(records) = batch.records(block) else {
            whole = false;
            return;
        };
        for (lsn, record) in records {
            if let Some((pid, _)) = record.page() {
                last.insert(pid, lsn);
            }
        }
    });
    let damaged = s.findings.iter().any(|f| f.lsn.0 < floor);
    (whole && !damaged && s.resumed == 0 && s.end.0 >= floor).then_some(last)
}

/// Returns the LSN of the last record of each page in the retained log of `img`, reading only
/// each page record's page. Fail with the first LSN before `end`, the end of the log recovery
/// kept, that the log cannot prove: a gap, an unreadable or missing block, or a batch whose records
/// do not decode.
pub(crate) fn retained<S: LogSource>(
    img: &mut LogImage<S>,
    end: Lsn,
) -> Result<BTreeMap<Pid, Lsn>, Lsn> {
    let mut last = BTreeMap::new();
    let Some(first) = img.first_batch() else {
        return if end == Lsn(0) { Ok(last) } else { Err(end) };
    };
    let block = img.block;
    let mut undecoded = None;
    let s = scan(img, first, Mode::Verify, |batch, _| {
        if batch.start >= end || undecoded.is_some() {
            return;
        }
        match batch.pages(block) {
            Ok(pages) => last.extend(pages.into_iter().map(|(lsn, pid)| (pid, lsn))),
            Err(_) => undecoded = Some(batch.start),
        }
    });
    let found = s.findings.iter().map(|f| f.lsn).filter(|&at| at < end);
    let short = (s.end < end).then_some(s.end);
    match found.chain(short).chain(undecoded).min() {
        Some(at) => Err(at),
        None => Ok(last),
    }
}
