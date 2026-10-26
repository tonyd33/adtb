use std::collections::BTreeMap;
use std::ops::Bound;

use crate::heap::Heap;
use crate::key::CtorId;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::run::{Error, Sum};

/// Returns what is wrong between the indexes of `sum` and its heaps: an entry whose constructor
/// has no heap in the sum, or whose TID names no version, dead mark or redirect of that heap.
pub fn verify<R: Runtime>(pager: &Pager<R>, sum: &Sum) -> Result<Vec<String>, Error> {
    let heaps: BTreeMap<CtorId, &Heap> = sum.heaps.iter().map(|(c, h)| (*c, &**h)).collect();
    let mut problems = Vec::new();
    for (i, index) in sum.indexes.iter().enumerate() {
        for e in index.range(pager, Bound::Unbounded, Bound::Unbounded)? {
            match heaps.get(&e.ctor) {
                None => problems.push(format!("index {i} has an entry of {:?}", e.ctor)),
                Some(h) if !h.holds(pager, e.tid)? => problems.push(format!(
                    "index {i} entry {:?} names {:?}, which holds nothing",
                    e.key, e.tid
                )),
                Some(_) => {}
            }
        }
    }
    Ok(problems)
}
