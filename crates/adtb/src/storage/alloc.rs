use std::collections::BTreeSet;

use crate::runtime::Runtime;

use super::page::{self, FIRST_EDITABLE, HEADER_LEN, PageKind};
use super::pager::{PageError, Pager};
use super::record::Edit;
use super::{Lsn, Pid};

/// Unchanged runs shorter than this between two changed ones are logged as changed.
const MERGE_GAP: usize = 16;

/// Returns the edits that turn `old` into `new`.
pub(crate) fn diff(old: &[u8], new: &[u8]) -> Vec<Edit> {
    let mut edits: Vec<Edit> = Vec::new();
    let mut i = 0;
    while i < new.len() {
        if old[i] == new[i] {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i + 1;
        let mut j = end;
        while j < new.len() && j - end < MERGE_GAP {
            if old[j] != new[j] {
                end = j + 1;
            }
            j += 1;
        }
        edits.push(Edit {
            offset: start as u32,
            bytes: new[start..end].to_vec(),
        });
        i = end;
    }
    edits
}

impl<R: Runtime> Pager<R> {
    /// Returns the number of pages one allocation bitmap page covers.
    fn bitmap_span(&self) -> u64 {
        ((self.page_size - HEADER_LEN) * 8) as u64
    }

    /// Returns the bitmap page that covers group `g` of [`Self::bitmap_span`] pages.
    fn bitmap_page(&self, g: u64) -> Pid {
        Pid((g * self.bitmap_span()).max(Pid::FIRST_DATA.0))
    }

    /// Modify `pids` together through `f`, which sees every page at once, and log what changed.
    ///
    /// Returns the LSN of the last record logged. Fails, calling nothing, if a page is damaged.
    ///
    /// Panics if `f` changes a page's LSN or checksum, or as [`Pager::change`] does.
    pub fn update<F>(&mut self, pids: &[Pid], f: F) -> Result<Lsn, PageError>
    where
        F: FnOnce(&mut [&mut [u8]]),
    {
        let pinned: BTreeSet<Pid> = pids.iter().copied().collect();
        assert_eq!(
            pinned.len(),
            pids.len(),
            "a page appears twice in one update"
        );
        for &pid in pids {
            self.fetch(pid, &pinned)?;
        }
        let mut copies: Vec<Vec<u8>> = pids.iter().map(|p| self.frames[p].page.to_vec()).collect();
        let mut views: Vec<&mut [u8]> = copies.iter_mut().map(|c| c.as_mut_slice()).collect();
        f(&mut views);
        let changes: Vec<(Pid, Vec<Edit>)> = pids
            .iter()
            .zip(&copies)
            .map(|(pid, new)| (*pid, diff(&self.frames[pid].page, new)))
            .filter(|(_, edits)| !edits.is_empty())
            .collect();
        self.change(&changes)
    }

    /// Allocate a free page, set its header to `format`, `kind` and `owner`, and zero its contents.
    ///
    /// Fails, allocating nothing, if an allocation bitmap page it reads is damaged. A free page
    /// found damaged is marked allocated, so it is never handed out, and the search goes on.
    ///
    /// Postconditions:
    ///
    /// - The page was free, and is now marked allocated in the same change that formats it.
    /// - Its records lie in one batch with the bitmap's, so a crash keeps both or neither.
    pub fn allocate(&mut self, format: u16, kind: PageKind, owner: u32) -> Result<Pid, PageError> {
        let span = self.bitmap_span();
        let mut g = self.alloc_group;
        loop {
            let map = self.bitmap_page(g);
            let first = g * span;
            let bits = self.read(map)?[HEADER_LEN..].to_vec();
            let taken = |pid: u64| {
                let i = (pid - first) as usize;
                pid < Pid::FIRST_DATA.0 || pid == map.0 || bits[i / 8] >> (i % 8) & 1 == 1
            };
            let Some(pid) = (first..first + span).find(|&p| !taken(p)) else {
                g += 1;
                continue;
            };
            self.alloc_group = g;
            let pid = Pid(pid);
            let mut format_bytes = page::header_edit(format, kind, owner, 0);
            format_bytes.resize(self.page_size - FIRST_EDITABLE, 0);
            let mut marked = bits.clone();
            for p in [map.0, pid.0] {
                let i = (p - first) as usize;
                marked[i / 8] |= 1 << (i % 8);
            }
            let mut map_edits = page::header_edit(1, PageKind::AllocationBitmap, 0, 0);
            map_edits.extend_from_slice(&marked);
            let changes = vec![
                (
                    map,
                    vec![Edit {
                        offset: FIRST_EDITABLE as u32,
                        bytes: map_edits,
                    }],
                ),
                (
                    pid,
                    vec![Edit {
                        offset: FIRST_EDITABLE as u32,
                        bytes: format_bytes,
                    }],
                ),
            ];
            match self.change(&changes) {
                Ok(_) => return Ok(pid),
                Err(e) if e.pid == pid => {
                    self.change(&changes[..1])?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Mark `pid` free. Its contents stay until it is allocated again.
    ///
    /// Preconditions: nothing refers to `pid`, and it is allocated.
    pub fn free(&mut self, pid: Pid) -> Result<Lsn, PageError> {
        let span = self.bitmap_span();
        let g = pid.0 / span;
        let map = self.bitmap_page(g);
        assert!(
            pid != map && pid >= Pid::FIRST_DATA,
            "{pid:?} is not an allocatable page"
        );
        let i = (pid.0 - g * span) as usize;
        let lsn = self.update(&[map], |pages| {
            let byte = &mut pages[0][HEADER_LEN + i / 8];
            assert!(*byte >> (i % 8) & 1 == 1, "{pid:?} is not allocated");
            *byte &= !(1 << (i % 8));
        })?;
        self.alloc_group = self.alloc_group.min(g);
        Ok(lsn)
    }
}
