use crate::key::{Tid, Value};
use crate::mvcc::Xid;
use crate::runtime::Runtime;
use crate::storage::{Pager, Pid};

use super::super::page::HeapPageMut;
use super::super::row::Pointer;
use super::super::stage::Stage;
use super::super::version::{Flags, Header};
use super::super::write::{Failed, HeapWrite, Rank, Took};
use super::super::{Abandons, Error};
use super::{Heap, Judge, Put, tid, version_bytes};

/// A version ready to put, and the pointers of its out-of-line values.
type Ready = (Vec<u8>, Vec<Pointer>);

impl Heap {
    /// Insert the rows of `rows` as `xid`, in order, each as [`Heap::insert_pruning`] inserts one,
    /// writing the versions one page takes in turn in one update, and call `placed` with each row
    /// and its TID once its page is written. Stop at the first error `rows` or `placed` gives,
    /// taking nothing back, as `_writer` does.
    ///
    /// Postcondition: a row is taken from `rows` only once every row before it is encoded, and at
    /// most a page of encoded versions waits to be put.
    pub fn insert_all_pruning<R: Runtime, V: AsRef<[Value]>, E: From<Error>>(
        &self,
        pager: &Pager<R>,
        xid: Xid,
        rows: impl IntoIterator<Item = Result<V, E>>,
        judge: &mut Judge<'_>,
        mut placed: impl FnMut(V, Tid) -> Result<(), E>,
        _writer: &impl Abandons,
    ) -> Result<(), E> {
        let mut rows = rows.into_iter();
        let mut ready: Vec<Ready> = Vec::new();
        let mut waiting: Vec<V> = Vec::new();
        loop {
            let mut bytes: usize = ready.iter().map(|(v, _)| v.len()).sum();
            while bytes < self.page_size {
                let Some(row) = rows.next() else { break };
                let row = row?;
                let (body, held) = self.body(pager, xid, row.as_ref(), &[])?;
                let version = version_bytes(Header::new(xid), &body);
                bytes += version.len();
                ready.push((version, held));
                waiting.push(row);
            }
            let Some((first, rest)) = ready.split_first() else {
                return Ok(());
            };
            let (tids, committed) = self.put_run(pager, xid, first, rest, Some(&mut *judge))?;
            committed.map_err(Error::from)?;
            ready.drain(..tids.len());
            for (row, t) in waiting.drain(..tids.len()).zip(tids) {
                placed(row, t)?;
            }
        }
    }

    /// Put `first` on a page found for it, pruning by `judge` a page found without room before
    /// looking further, then each of `rest` in turn until one does not fit that page, all in one
    /// write. Returns the TIDs of the versions put and the commit's failure after its update wrote
    /// them, if it failed.
    pub(super) fn put_run<R: Runtime>(
        &self,
        pager: &Pager<R>,
        xid: Xid,
        (first, held): &Ready,
        rest: &[Ready],
        mut judge: Option<&mut Judge<'_>>,
    ) -> Result<(Vec<Tid>, Result<(), Failed>), Error> {
        loop {
            let pid = self.candidate(pager, first.len())?;
            let pre = self.preconvert(pager, pid, xid)?;
            let mut write = HeapWrite::new(pager);
            write.latch(&[(self, pid)])?;
            match self.try_put(write.stage(), pager, pid, first, Flags::NONE, pre)? {
                Put::Done(t, c) => {
                    let mut tids = vec![t];
                    let mut retain = [&held[..], &c.gained].concat();
                    for (version, held) in rest {
                        let Some(t) = put_staged(write.stage(), pager, pid, version)? else {
                            break;
                        };
                        tids.push(t);
                        retain.extend(held.iter().cloned());
                    }
                    let took = Took::Found(self, pid);
                    return match write.commit(took, &retain, &c.dropped).finish() {
                        Err(f) if !f.written() => Err(f.into()),
                        committed => Ok((tids, committed)),
                    };
                }
                Put::Stale => continue,
                Put::Full => {}
            }
            drop(write);
            if self.prune_full(pager, pid, judge.as_deref_mut())? {
                continue;
            }
            self.refused(pager, pid)?;
        }
    }
}

/// Put `version` on the staged copy of `pid`, a listed page of the newest format the stage
/// holds. Returns its TID, or `None` when the page has no room.
fn put_staged<'p, R: Runtime>(
    stage: &mut Stage<'p, R>,
    pager: &'p Pager<R>,
    pid: Pid,
    version: &[u8],
) -> Result<Option<Tid>, Error> {
    let mut page = stage.read(pager, Rank::Heap(pid))?;
    let Some(mut heap_page) = HeapPageMut::any(&mut page) else {
        return Ok(None);
    };
    let Some(slot) = heap_page.insert(version, Flags::NONE) else {
        return Ok(None);
    };
    stage.put(pager, Rank::Heap(pid), page.into_bytes())?;
    Ok(Some(tid(pid, slot)))
}
