//! The file settings the engine reads from `catalog::settings`, and the open-time settings it
//! shows in `sys::process_options`.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::RelId;

use crate::catalog::{Catalog, layout};
use crate::exec::{self, Val};
use crate::mvcc::Transactions;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::{OpenError, corrupt};

const HOUR: i64 = 3_600_000_000;

/// The file settings the engine acts on, each in microseconds or a count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Settings {
    /// The lease of a bundle whose header states none.
    pub(super) lease: i64,
    /// How long past its lease the primary still serves a bundle.
    pub(super) grace: i64,
    /// The reruns of a serialization failure when a call or block states no `retry n`.
    pub(super) retries: u64,
    /// The length of the retention window.
    pub(super) window: Duration,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            lease: 30 * 24 * HOUR,
            grace: HOUR,
            retries: 5,
            window: Duration::ZERO,
        }
    }
}

impl Settings {
    /// Returns the settings the rows of `catalog::settings` the latest commit holds state, each
    /// setting with no row at its default.
    pub(super) fn stored<R: Runtime>(
        catalog: &Catalog,
        pager: &Pager<R>,
        txns: &Transactions<R>,
    ) -> Result<Settings, OpenError> {
        let schema = catalog.schema();
        let rel = relation(schema);
        let mut out = Settings::default();
        for (c, row) in catalog.committed_rows(pager, txns, rel).map_err(corrupt)? {
            let fields = layout::row(schema, rel, c)
                .iter()
                .zip(&row)
                .map(|((_, t), v)| exec::from_stored(&exec::conv(schema, t), v))
                .collect();
            out.set(schema, &Val::ctor(c, fields));
        }
        Ok(out)
    }

    /// Takes the setting `row`, a row of `catalog::settings`, in place of the one it replaces.
    pub(super) fn set(&mut self, schema: &Schema, row: &Val) {
        let Val::Ctor(c, fields) = row else {
            return;
        };
        let def = schema.relation(relation(schema)).elem_def;
        let name = schema.ty(def).ctors()[*c as usize].name.as_str();
        match (name, fields.get(1)) {
            ("DefaultLease", Some(Val::Duration(d))) => self.lease = *d,
            ("LeaseGrace", Some(Val::Duration(d))) => self.grace = *d,
            ("RetryLimit", Some(Val::Int(n))) => self.retries = u64::try_from(*n).unwrap_or(0),
            ("RetentionWindow", Some(Val::Duration(d))) => {
                self.window = Duration::from_micros(u64::try_from(*d).unwrap_or(0));
            }
            _ => {}
        }
    }
}

/// Returns the relation `catalog::settings` of `schema`.
fn relation(schema: &Schema) -> RelId {
    engine_relation(schema, "catalog", "settings")
}

/// Returns the relation `module::name` the engine declares in `schema`.
fn engine_relation(schema: &Schema, module: &str, name: &str) -> RelId {
    let m = schema
        .module_named(module)
        .unwrap_or_else(|| panic!("the engine declares module `{module}`"));
    match schema.module(m).values.get(name) {
        Some(ValueDef::Relation(r)) => *r,
        _ => panic!("the engine declares `{module}::{name}`"),
    }
}

/// One open-time setting as `sys::process_options` shows it.
pub(super) struct Shown {
    pub(super) name: &'static str,
    pub(super) value: String,
    /// Whether the engine derived the value, the program giving none.
    pub(super) derived: bool,
}

/// Returns the rows of `sys::process_options` of `schema` showing `shown`.
pub(super) fn process_options(schema: &Schema, shown: &[Shown]) -> BTreeMap<RelId, BTreeSet<Val>> {
    let rel = engine_relation(schema, "sys", "process_options");
    let rows = shown
        .iter()
        .enumerate()
        .map(|(i, s)| {
            Val::ctor(
                0,
                vec![
                    Val::Int(i64::try_from(i + 1).unwrap_or(i64::MAX)),
                    Val::text(s.name),
                    Val::text(&s.value),
                    Val::bool(s.derived),
                ],
            )
        })
        .collect();
    BTreeMap::from([(rel, rows)])
}
