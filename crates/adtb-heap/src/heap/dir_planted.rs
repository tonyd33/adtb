#[cfg(any(test, feature = "testing"))]
use std::cell::Cell;

#[cfg(any(test, feature = "testing"))]
use crate::runtime::Runtime;
#[cfg(any(test, feature = "testing"))]
use crate::storage::{PageError, Pager};

#[cfg(any(test, feature = "testing"))]
use super::{HOLE, Space, count, entry, level, set_word, touched};

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static FANOUT: Cell<Option<u16>> = const { Cell::new(None) };
}

/// Give each directory of a heap or store the calling thread creates from now on `fanout`
/// entries a page, or as many as a page holds when `None`.
#[cfg(any(test, feature = "testing"))]
pub fn create_directories_with_fanout(fanout: Option<u16>) {
    FANOUT.with(|f| f.set(fanout));
}

/// Returns the entries a page of a new directory of a heap or store holds.
pub fn fanout(page_size: usize) -> u16 {
    #[cfg(any(test, feature = "testing"))]
    if let Some(f) = FANOUT.with(Cell::get) {
        return f;
    }
    super::max_fanout(page_size)
}

#[cfg(any(test, feature = "testing"))]
impl Space {
    /// Give the first entry of the root whose child holds a page with room the category of a
    /// full page, as a crash between a change below and the raise of the entry above leaves it.
    /// Returns whether it found one.
    pub fn unsettle<R: Runtime>(&self, pager: &Pager<R>) -> Result<bool, PageError> {
        let Some(root) = self.root else {
            return Ok(false);
        };
        let node = pager.read_formatted(root)?.to_vec();
        if level(&node) == 0 {
            return Ok(false);
        }
        for i in 0..count(&node) {
            let (child, cat) = entry(&node, i);
            if child == HOLE || cat == 0 {
                continue;
            }
            let lowered = touched(&node, i);
            pager.update(&[root], |p| set_word(p[0], i, child, 0, lowered))?;
            return Ok(true);
        }
        Ok(false)
    }
}
