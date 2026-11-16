//! The file settings the engine reads from `catalog::settings`, and the open-time settings it
//! shows in `sys::process_options`.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use adtb_lang::ids::engine_rel;
use adtb_lang::schema::Schema;
use adtb_lang::types::RelId;

use crate::catalog::codec::{Codec, SettingRow};
use crate::catalog::{Catalog, engine_relation};
use crate::exec::{F64, Val};
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
    /// The period of the background scrub, `None` for no scrub.
    pub(super) scrub: Option<Duration>,
    /// The share of time vacuum and the scrub may spend working: the least of the fractions of
    /// `VacuumBudget`.
    pub(super) budget: F64,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            lease: 30 * 24 * HOUR,
            grace: HOUR,
            retries: 5,
            window: Duration::ZERO,
            scrub: Some(Duration::from_secs(7 * 24 * 3600)),
            budget: F64::new(0.1),
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
        let codec = Codec::<SettingRow>::new(catalog.schema());
        let mut out = Settings::default();
        for row in catalog.committed(pager, txns, &codec).map_err(corrupt)? {
            out.set(&row);
        }
        Ok(out)
    }

    /// Take the setting `row` in place of the one it replaces.
    pub(super) fn set(&mut self, row: &SettingRow) {
        use SettingRow as S;
        match *row {
            S::DefaultLease(d) => self.lease = d,
            S::LeaseGrace(d) => self.grace = d,
            S::RetryLimit(n) => self.retries = u64::try_from(n).unwrap_or(0),
            S::RetentionWindow(d) => {
                self.window = Duration::from_micros(u64::try_from(d).unwrap_or(0));
            }
            S::ScrubInterval(every) => {
                self.scrub = every.map(|d| Duration::from_micros(u64::try_from(d).unwrap_or(0)));
            }
            S::VacuumBudget { io, cpu } => self.budget = F64::new(io.get().min(cpu.get())),
            S::StatementLimit(_)
            | S::TransactionLimit(_)
            | S::IdleLimit(_)
            | S::StreamIdleLimit(_)
            | S::RequestWindow(_)
            | S::SafePointDelay(_)
            | S::ReplicaWait(_)
            | S::FeedRetention(_)
            | S::StatsWindow(_)
            | S::StatsRetention(_)
            | S::WaitSampling { .. }
            | S::ConflictLog(_)
            | S::BlockingNoticePeriod(_)
            | S::ReplicaVacuumWait(_) => {}
        }
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
    let rel = engine_relation(schema, engine_rel::PROCESS_OPTIONS);
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
