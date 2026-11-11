//! A call that fails returns its own failure once the abort that ends its transaction is
//! recorded, and the abort record's failure when it is not.

use std::collections::BTreeSet;

use adtb_harness::process::run;
use adtb_sim::{Config, Sim};

use crate::runtime::Runtime as _;
use crate::storage::{Damage, PageError, Quarantine};
use crate::test_world as harness;
use crate::test_world::load::{self as open, options};
use crate::testing::Plant;
use crate::{CallOptions, Error, FatalReason, Log, RejectReason, TxnCallOptions, TxnOptions};
use crate::{mvcc, write};

const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
";

const RENAME: &str = r#"update u in users where u.name == "a" set User { u with name: "b" }"#;

const FAILING: &str = r#"(insert users { name: "a" }, fail "no")"#;

/// Run `FAILING` on a fresh database after `before`. Returns how the call ended and how many more
/// abort records went unrecorded.
fn failing_write(
    seed: u64,
    before: impl FnOnce(&crate::Database<adtb_sim::SimRuntime>),
) -> (Result<(), Error>, usize) {
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &options(2));
        before(&db);
        let unrecorded = db.engine.db.txns().unrecorded_aborts();
        let got = db
            .query_text(FAILING, &[], &CallOptions::default())
            .map(drop);
        let counted = db.engine.db.txns().unrecorded_aborts() - unrecorded;
        db.close().expect("the database closes");
        (got, counted)
    })
}

#[test]
fn a_failing_write_returns_its_failure_once_its_abort_is_recorded() {
    let failed = Error::Rejected {
        reason: RejectReason::UserError {
            message: "no".to_owned(),
        },
    };
    assert_eq!((Err(failed), 0), failing_write(4, |_| {}));
}

#[test]
fn a_failing_write_whose_abort_record_fails_returns_the_records_failure_and_counts_it() {
    let mut status = None;
    let got = failing_write(4, |db| {
        let pages = db.testing().commit_log().status().clone();
        let page = *pages.values().next_back().expect("a status page");
        db.testing()
            .plant(Plant::Refuse {
                page,
                damage: Damage::Unreadable,
            })
            .expect("planted");
        status = Some(page);
    });
    let unreadable = PageError {
        pid: status.expect("a status page"),
        damage: Damage::Unreadable,
    };
    let failed = Error::Fatal {
        reason: FatalReason::Storage {
            detail: format!("{:?}", mvcc::Error::Corrupt(unreadable)),
        },
    };
    assert_eq!((Err(failed), 1), got);
}

#[test]
fn a_transaction_ended_by_a_failure_its_abort_cannot_record_answers_later_calls_with_it() {
    let mut sim = Sim::new(7, Config::default());
    run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &options(2));
        let opts = TxnCallOptions::default();
        db.query_text(
            r#"insert users { name: "a" }"#,
            &[],
            &CallOptions::default(),
        )
        .expect("a row");
        let mut txn = db.begin(&TxnOptions::default()).expect("a transaction");
        let pages = db.testing().commit_log().status().clone();
        let page = *pages.values().next_back().expect("a status page");
        let refused = Plant::Refuse {
            page,
            damage: Damage::Unreadable,
        };
        db.testing().plant(refused).expect("planted");
        let unreadable = PageError {
            pid: page,
            damage: Damage::Unreadable,
        };
        let storage = |detail| Error::Fatal {
            reason: FatalReason::Storage { detail },
        };
        let record = storage(format!("{:?}", mvcc::Error::Corrupt(unreadable)));
        let cause = storage(format!("{unreadable:?}"));
        let unrecorded = db.engine.db.txns().unrecorded_aborts();
        let failing = txn.query_text(RENAME, &[], &opts).map(drop);
        assert_eq!(Err(record), failing, "the failing call");
        assert_eq!(1, db.engine.db.txns().unrecorded_aborts() - unrecorded);
        let later = txn.query_text("1", &[], &opts).map(drop);
        assert_eq!(Err(cause.clone()), later, "a later call");
        assert_eq!(Err(cause), txn.commit(None).map(drop), "the commit");
        db.close().expect("the database closes");
    });
}

/// Returns bundle `name` pinned at [`APP`], leased for an hour.
fn leased(name: &str) -> crate::Registration {
    let file = adtb_lang::parse::parse_migration(APP, adtb_lang::parse::Options::default())
        .value
        .expect("the migration parses");
    let source = adtb_lang::evolve::Source::default()
        .migrate(&file, APP)
        .expect("the migration applies");
    let schema = source.schema().expect("the schema checks");
    let hash = crate::catalog::version_hash(&schema, &source);
    let hash: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    crate::Registration::bundle(format!(
        "bundle {name} pins 1 ({hash}) edition \"2026-10\" lease 1h\nquery hello () -> Text = \"hi\"\n"
    ))
    .expect("the bundle parses")
}

#[test]
fn damage_met_writing_a_lapsed_lease_fails_as_storage_and_marks_nothing() {
    let mut sim = Sim::new(17, Config::default());
    run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &options(2));
        let opts = CallOptions::default();
        db.query_text("set setting LeaseGrace = { grace: 60s }", &[], &opts)
            .expect("the grace is set");
        let owner = db.role_authority().session("owner").expect("the owner");
        db.role_authority()
            .register(&owner, &leased("s-1.0"))
            .expect("registered");
        adtb_harness::common::sleep(&rt, &mut rt.io(), std::time::Duration::from_secs(7200));
        let rows = crate::testing::Object::CatalogRows;
        let pages = db.testing().pages(rows).expect("the rows heap's pages");
        for &page in &pages {
            let refused = Plant::Refuse {
                page,
                damage: Damage::Checksum,
            };
            db.testing().plant(refused).expect("planted");
        }
        let unrecorded = db.engine.db.txns().unrecorded_aborts();
        let got = db
            .query_text("set setting LeaseGrace = { grace: 100h }", &[], &opts)
            .map(drop);
        let storage = |pid| {
            Err(Error::Fatal {
                reason: FatalReason::Storage {
                    detail: format!(
                        "{:?}",
                        PageError {
                            pid,
                            damage: Damage::Checksum
                        }
                    ),
                },
            })
        };
        assert!(
            pages.iter().any(|&p| storage(p) == got),
            "{got:?} names no page of {pages:?}"
        );
        assert_eq!(BTreeSet::new(), db.testing().probe().marks());
        assert_eq!(0, db.engine.db.txns().unrecorded_aborts() - unrecorded);
        db.close().expect("the database closes");
    });
}

#[test]
fn a_deployment_row_that_does_not_decode_fails_a_dump_as_storage() {
    let mut sim = Sim::new(18, Config::default());
    run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &options(2));
        let bad = crate::key::Value::Sum {
            ctor: 13,
            fields: vec![
                crate::key::Value::I64(1),
                crate::key::Value::I64(1),
                crate::key::Value::I64(7),
                crate::key::Value::Text("t".to_owned()),
            ],
        };
        db.testing()
            .plant(Plant::Insert {
                heap: crate::testing::Object::CatalogRows,
                rows: vec![vec![bad]],
            })
            .expect("planted");
        let got = db
            .query_text("dump to \"d\"", &[], &CallOptions::default())
            .map(drop);
        let storage = Error::Fatal {
            reason: FatalReason::Storage {
                detail: "registration origin 7".to_owned(),
            },
        };
        assert_eq!(Err(storage), got);
        assert_eq!(BTreeSet::new(), db.testing().probe().marks());
        db.close().expect("the database closes");
    });
}

#[test]
fn a_call_meeting_a_closed_pager_is_rejected_as_closed() {
    let mut sim = Sim::new(19, Config::default());
    run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &options(2));
        db.engine.pager.close();
        let got = db
            .query_text(
                r#"insert users { name: "a" }"#,
                &[],
                &CallOptions::default(),
            )
            .map(drop);
        assert_eq!(
            Err(Error::Rejected {
                reason: RejectReason::Closed
            }),
            got
        );
        *db.engine.open.write(&rt) = false;
    });
}

#[test]
fn a_closed_pager_is_rejected_as_closed_once_the_database_is_marked() {
    let mut sim = Sim::new(20, Config::default());
    run(&mut sim, |rt| {
        let db = open::opened(&rt, harness::NAME, &Log::new([APP]), &options(2));
        db.testing()
            .plant(Plant::Mark(Quarantine::Database))
            .expect("the database is marked");
        let schema = crate::catalog::engine_schema();
        let reply = |e| {
            let mapped = db.engine.db.fail(write::Error::Transactions(e));
            Error::of_write(&db.engine.db, &schema, super::TxnId(7), mapped)
        };
        assert_eq!(
            Error::Rejected {
                reason: RejectReason::Closed
            },
            reply(mvcc::Error::Closed)
        );
        assert_eq!(
            Error::Fatal {
                reason: FatalReason::Quarantined {
                    what: "the database".into()
                }
            },
            reply(mvcc::Error::Exhausted)
        );
        db.close().expect("the database closes");
    });
}
