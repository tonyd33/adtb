//! A call that fails returns its own failure once the abort that ends its transaction is
//! recorded, and the abort record's failure when it is not.

use adtb_harness::process::run;
use adtb_sim::{Config, Sim};

use crate::storage::{Damage, PageError};
use crate::test_world as harness;
use crate::test_world::load::{self as open, options};
use crate::testing::Plant;
use crate::{CallOptions, Error, FatalReason, Log, RejectReason, TxnCallOptions, TxnOptions};

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
            detail: format!("{:?}", crate::mvcc::Error::Corrupt(unreadable)),
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
        let record = storage(format!("{:?}", crate::mvcc::Error::Corrupt(unreadable)));
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
