use std::collections::{BTreeMap, BTreeSet};

use crate::heap::Heap;
use crate::key::CtorId;
use crate::runtime::Runtime;
use crate::storage::Pager;
use crate::verify::{self, entries};

use super::run::{Error, Sum};

/// Returns what is wrong between the indexes of `sum` and its heaps: an entry whose constructor
/// has no heap in the sum, or whose TID names no version, dead mark or redirect of that heap.
pub fn verify<R: Runtime>(pager: &Pager<R>, sum: &Sum) -> Result<Vec<String>, Error> {
    let heaps: BTreeMap<CtorId, &Heap> = sum.heaps.iter().map(|(c, h)| (*c, *h)).collect();
    let mut problems = Vec::new();
    for (i, index) in sum.indexes.iter().enumerate() {
        let found = entries(pager, &heaps, index, &BTreeSet::new()).map_err(|e| match e {
            verify::Error::Page(e) => Error::Page(e),
            verify::Error::Index(e) => Error::Index(e),
        })?;
        problems.extend(
            found
                .into_iter()
                .map(|(pid, what)| format!("index {i} at {pid:?}: {what}")),
        );
    }
    Ok(problems)
}
