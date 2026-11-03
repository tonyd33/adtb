use crate::storage::Pid;
use crate::{btree, exec};

use super::super::{Error, detail};
use super::{Rot, Rot as R};

/// Returns the case that follows `r`, the first when `r` is `None`, and the text it reads as.
/// A walk from `None` reaches every variant.
#[rustfmt::skip]
fn after(r: Option<&Rot>) -> Option<(Rot, &'static str)> {
    Some(match r {
        None => (R::Index(btree::Error::Malformed(Pid(4))), "Malformed(Pid(4))"),
        Some(R::Index(_)) => (R::NoKeyIndex("us".into()), "relation us has no key index"),
        Some(R::NoKeyIndex(_)) => (R::Exec(exec::Rot::SpillCut), "a spilled record is cut short"),
        Some(R::Exec(_)) => return None,
    })
}

#[test]
fn each_damage_reads_as_its_text() {
    let mut seen = Vec::new();
    for (rot, text) in std::iter::successors(after(None), |(r, _)| after(Some(r))) {
        assert!(!seen.contains(&rot), "the walk returns to {rot:?}");
        assert_eq!(text, rot.to_string(), "{rot:?}");
        seen.push(rot);
    }
}

#[test]
fn detail_reads_a_damage_cause_as_the_debug_of_its_text() {
    let text = "Corrupt(\"relation us has no key index\")";
    assert_eq!(text, detail(&Error::from(Rot::NoKeyIndex("us".into()))));
}
