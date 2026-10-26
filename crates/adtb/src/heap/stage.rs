use crate::runtime::Runtime;
use crate::storage::{self, FIRST_EDITABLE, PageError, Pager, Pid};

/// Copies of pages being changed together, written back by one pager update.
#[derive(Debug, Default)]
pub(super) struct Stage {
    pages: Vec<(Pid, Vec<u8>)>,
}

impl Stage {
    pub fn new() -> Stage {
        Stage::default()
    }

    /// Returns a copy of `pid` as staged, or as the pager holds it when it is not staged.
    pub fn read<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<Vec<u8>, PageError> {
        match self.pages.iter().find(|(p, _)| *p == pid) {
            Some((_, bytes)) => Ok(bytes.clone()),
            None => Ok(pager.read(pid)?.to_vec()),
        }
    }

    /// Returns the staged copy of `pid`, staging it first if needed.
    pub fn page<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        pid: Pid,
    ) -> Result<&mut Vec<u8>, PageError> {
        let i = match self.pages.iter().position(|(p, _)| *p == pid) {
            Some(i) => i,
            None => {
                let bytes = pager.read(pid)?.to_vec();
                self.pages.push((pid, bytes));
                self.pages.len() - 1
            }
        };
        Ok(&mut self.pages[i].1)
    }

    pub fn put(&mut self, pid: Pid, bytes: Vec<u8>) {
        match self.pages.iter_mut().find(|(p, _)| *p == pid) {
            Some((_, b)) => *b = bytes,
            None => self.pages.push((pid, bytes)),
        }
    }

    /// Write every staged page back in one pager update, so a crash keeps all of them or none.
    pub fn commit<R: Runtime>(self, pager: &Pager<R>) -> Result<(), PageError> {
        if self.pages.is_empty() {
            return Ok(());
        }
        let pids: Vec<Pid> = self.pages.iter().map(|(p, _)| *p).collect();
        pager.update(&pids, |views| {
            for (view, (pid, bytes)) in views.iter_mut().zip(&self.pages) {
                assert_eq!(
                    storage::lsn(view),
                    storage::lsn(bytes),
                    "{pid:?} changed while it was staged"
                );
                view[FIRST_EDITABLE..].copy_from_slice(&bytes[FIRST_EDITABLE..]);
            }
        })?;
        Ok(())
    }
}
