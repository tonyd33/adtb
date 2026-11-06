//! The page LSN map: the last LSN of each page whose records the log no longer holds, in a radix
//! tree of `height` levels. A leaf holds one LSN per page; an interior page holds, for each
//! child, its pid and the least LSN it may have. A zero word holds nothing.

use std::collections::BTreeMap;

use crate::runtime::Runtime;

use super::block::{LogImage, LogSource, Mode, scan};
use super::page::{self, HEADER_LEN, PageKind};
use super::pager::{Damage, PageError, Pager};
use super::quarantine::Quarantine;
use super::record::{MapRoot, Record};
use super::{Lsn, Pid};

const FORMAT: u16 = 1;

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
        self.quarantine(Quarantine::Database).ok();
        Err(refused)
    }

    /// Check the map's root against the LSN the checkpoint or the last fold gave it, quarantining
    /// the database when it is refused.
    pub(super) fn check_map(&self) {
        let map = self.meta().map;
        if let Some(m) = map {
            drop(self.map_page(m.root, m.lsn));
        }
    }

    /// Returns the LSN the map holds for `pid`, refusing a map page whose LSN is below the one
    /// its parent gives it and quarantining the database.
    pub(crate) fn mapped(&self, pid: Pid) -> Result<Option<Lsn>, PageError> {
        let Some(m) = self.meta().map else {
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
        let map = self.meta().map;
        if let Some(m) = map {
            self.subtree(m.root, m.lsn, m.height, &mut out)?;
        }
        Ok(out)
    }

    fn subtree(
        &self,
        pid: Pid,
        expected: Lsn,
        level: u32,
        out: &mut Vec<Pid>,
    ) -> Result<(), PageError> {
        out.push(pid);
        let page = self.map_page(pid, expected)?;
        if level == 1 {
            return Ok(());
        }
        for slot in 0..self.fanout() as usize {
            let child = Pid(word(&page, 2 * slot));
            if child != Pid(0) {
                self.subtree(child, Lsn(word(&page, 2 * slot + 1)), level - 1, out)?;
            }
        }
        Ok(())
    }

    /// Raise the map's entry of each page of `last` to its LSN, adding levels above the root
    /// until it reaches the highest. A page at or past the file's end no longer exists, and is
    /// dropped.
    pub(crate) fn fold(&self, last: &BTreeMap<Pid, Lsn>) -> Result<(), PageError> {
        let end = self.file_pages();
        let entries: Vec<(u64, Lsn)> = last
            .iter()
            .filter(|(p, _)| p.0 < end)
            .map(|(p, l)| (p.0, *l))
            .collect();
        let Some(&(top, _)) = entries.last() else {
            return Ok(());
        };
        let map = self.meta().map;
        let mut map = match map {
            Some(m) => m,
            None => self.grown(None)?,
        };
        while self.reach(map.height) <= top {
            map = self.grown(Some(map))?;
        }
        self.fold_into(map.root, map.lsn, map.height, &entries)?;
        let lsn = page::lsn(&self.read(map.root)?);
        self.meta().map = Some(MapRoot { lsn, ..map });
        Ok(())
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
