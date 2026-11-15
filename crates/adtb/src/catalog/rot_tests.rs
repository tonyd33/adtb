use crate::exec;
use crate::key::Value;
use crate::storage::Pid;

use super::super::Broken;
use super::super::ids::CatIndexId;
use super::{Rot, Rot as R};

/// Returns the case that follows `r`, the first when `r` is `None`, and the text it reads as.
/// A walk from `None` reaches every variant.
#[rustfmt::skip]
fn after(r: Option<&Rot>) -> Option<(Rot, &'static str)> {
    let row = || vec![Value::I64(1)];
    Some(match r {
        None => (R::NegativeId(-3), "a roots row holds negative id -3"),
        Some(R::NegativeId(_)) => (R::NotAnId(Value::Text("t".into())), "a roots row holds Text(\"t\") for an id"),
        Some(R::NotAnId(_)) => (R::RootsRow(row()), "a roots row is [I64(1)]"),
        Some(R::RootsRow(_)) => (R::HeapIdWide(9), "heap id 9 is past 32 bits"),
        Some(R::HeapIdWide(_)) => (R::OrderKey(vec![0]), "order key [0] is not valid"),
        Some(R::OrderKey(_)) => (R::RootsCtor { ctor: 9, fields: row() }, "a roots row of constructor 9 holds [I64(1)]"),
        Some(R::RootsCtor { .. }) => (R::PendingRow { ctor: 9, fields: row() }, "a roots row of constructor 9 holds [I64(1)]"),
        Some(R::PendingRow { .. }) => (R::NoOrderKey(9), "constructor 9 has no stored order key"),
        Some(R::NoOrderKey(_)) => (R::OrderKeyOrder(9), "the order key of constructor 9 does not follow the one before it"),
        Some(R::OrderKeyOrder(_)) => (R::Unfinished(Vec::new()), "[]"),
        Some(R::Unfinished(_)) => (R::MarkedNoCatalog, "a marked database has no whole catalog and writes none"),
        Some(R::MarkedNoCatalog) => (R::RootPage { pid: Pid(4), found: (2, 3) }, "the header's catalog root Pid(4) has kind and format (2, 3)"),
        Some(R::RootPage { .. }) => (R::KeptIndexes(Box::new(Broken::PendingRelations)), "the schema keeping pending drops has the relations of the version"),
        Some(R::KeptIndexes(_)) => (R::IndexKeying(Box::new(super::Error::Missing("m".into()))), "Missing(\"m\")"),
        Some(R::IndexKeying(_)) => (R::CatalogRow { row: "catalog::roles.Role".into(), detail: "holds I64(1) in field `name`".into() }, "a row of catalog::roles.Role holds I64(1) in field `name`"),
        Some(R::CatalogRow { .. }) => (R::UntaggedGrant(9), "grant row 9 records no grant of the log"),
        Some(R::UntaggedGrant(_)) => (R::KeptRow(row()), "a kept row holds [I64(1)]"),
        Some(R::KeptRow(_)) => (R::KeptFormat(9), "a kept row has format 9; this engine reads format 2"),
        Some(R::KeptFormat(_)) => (R::DroppedTwice { module: "app".into(), name: "t".into(), first: 2, second: 3 }, "app::t is held as dropped at versions 2 and 3"),
        Some(R::DroppedTwice { .. }) => (R::DropVersion(9), "a pending drop names version 9, which the log does not hold"),
        Some(R::DropVersion(_)) => (R::UniqueNotDropped { index: CatIndexId::from_raw(5), version: 9 }, "unique 5 is held as dropped at version 9, which does not drop it"),
        Some(R::UniqueNotDropped { .. }) => (R::Origin(9), "registration origin 9"),
        Some(R::Origin(_)) => (R::DeploymentRow { ctor: 9, fields: row() }, "a deployment row of constructor 9 holds [I64(1)]"),
        Some(R::DeploymentRow { .. }) => (R::NoMigration(9), "missing migration 9"),
        Some(R::NoMigration(_)) => (R::Exec(exec::Rot::SpillCut), "a spilled record is cut short"),
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
fn detail_reads_an_internal_or_damage_cause_as_the_debug_of_its_text() {
    let corrupt = super::Error::from(Rot::UntaggedGrant(9));
    assert_eq!(
        "Corrupt(\"grant row 9 records no grant of the log\")",
        super::detail(&corrupt)
    );
    let internal = super::Error::from(Broken::PendingRelations);
    let text = "Internal(\"the schema keeping pending drops has the relations of the version\")";
    assert_eq!(text, super::detail(&internal));
}
