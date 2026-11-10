use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};

use adtb_sim::{Config, Outcome, SimRuntime};
use adtb_testkit::fixture::Fixture;

use crate::mvcc::{Status, Xid};
use crate::test_world::write_path::{self, WritePath};

use super::Database;
use super::reindex::{OFFLINE_GATE, VACUUM_HOLDER};

const ITEMS: &str =
    "type Item = { id: Key items, n: I64 }\nrelation items : Item key id generate sequence\n";

/// What a database's transactions hold: the serializer's entries, the readers' snapshots, the
/// ids the waits hold running and the ids the transactions hold running.
type Holding = (usize, usize, BTreeSet<Xid>, BTreeSet<Xid>);

fn holding(db: &Database<SimRuntime>) -> Holding {
    let gates = [OFFLINE_GATE, VACUUM_HOLDER];
    let waits = db.waits_running().into_iter();
    (
        db.tracked(),
        db.readers_held(),
        waits.filter(|x| !gates.contains(x)).collect(),
        db.txns().running(),
    )
}

fn nothing() -> Holding {
    (0, 0, BTreeSet::new(), BTreeSet::new())
}

/// Run `body` over a write path of a table of items in a simulator of `seed`.
fn at(seed: u64, body: impl Fn(&Database<SimRuntime>)) {
    let f = Fixture::new(ITEMS);
    let mut sim = write_path::sim(seed, Config::default(), &f);
    let outcome = sim.run(|rt| body(WritePath::loaded(&rt, &f, None).db()));
    assert_eq!(Outcome::Finished, outcome, "seed {seed}");
}

#[test]
fn a_dropped_writer_releases_what_it_holds_and_records_its_abort() {
    at(1, |db| {
        let tx = db.begin().expect("a writer begins");
        let x = tx.xid().expect("a writer has an id");
        assert_eq!(
            (1, 0, BTreeSet::from([x]), BTreeSet::from([x])),
            holding(db)
        );
        drop(tx);
        assert_eq!(nothing(), holding(db));
        assert_eq!(Ok(Status::Aborted), db.txns().recorded(db.pager(), x));
        assert_eq!(1, db.take_ends_by_drop());
    });
}

#[test]
fn a_dropped_reader_releases_its_entry_and_snapshot() {
    at(2, |db| {
        let tx = db.begin_read(false).expect("a reader begins");
        assert_eq!((1, 1), (db.tracked(), db.readers_held()));
        drop(tx);
        assert_eq!(nothing(), holding(db));
        assert_eq!(1, db.take_ends_by_drop());
    });
}

#[test]
fn a_writer_dropped_while_its_thread_panics_releases_what_it_holds_and_records_nothing() {
    at(3, |db| {
        let mut began = None;
        let ran = catch_unwind(AssertUnwindSafe(|| {
            let tx = db.begin().expect("a writer begins");
            began = tx.xid();
            panic!("the injected failure");
        }));
        let message = ran.err().and_then(|p| p.downcast_ref::<&str>().copied());
        assert_eq!(Some("the injected failure"), message);
        let x = began.expect("a writer has an id");
        assert_eq!(nothing(), holding(db));
        assert_eq!(Ok(Status::InProgress), db.txns().recorded(db.pager(), x));
        assert_eq!(1, db.txns().unrecorded_aborts());
        assert_eq!(1, db.take_ends_by_drop());
    });
}

#[test]
fn a_writer_dropped_once_its_pager_closes_releases_what_it_holds_and_records_nothing() {
    at(4, |db| {
        let tx = db.begin().expect("a writer begins");
        db.pager().close();
        drop(tx);
        assert_eq!(nothing(), holding(db));
        assert_eq!(1, db.txns().unrecorded_aborts());
        assert_eq!(1, db.take_ends_by_drop());
    });
}

#[test]
fn a_commit_and_an_abort_end_their_transactions_by_hand() {
    at(5, |db| {
        let tx = db.begin().expect("a writer begins");
        db.commit(tx).expect("the writer commits");
        let tx = db.begin().expect("a writer begins");
        db.abort(tx).expect("the writer aborts");
        let tx = db.begin_read(false).expect("a reader begins");
        db.commit(tx).expect("the reader commits");
        assert_eq!(nothing(), holding(db));
        assert_eq!(0, db.take_ends_by_drop());
    });
}

/// Drop `tx` on a thread panicking with the injected failure.
fn drop_panicking<T: Send>(tx: T) {
    let ran = catch_unwind(AssertUnwindSafe(move || {
        let _tx = tx;
        panic!("the injected failure");
    }));
    let message = ran.err().and_then(|p| p.downcast_ref::<&str>().copied());
    assert_eq!(Some("the injected failure"), message);
}

#[test]
fn a_later_end_logs_without_a_reader_dropped_unlogged() {
    at(6, |db| {
        let w = db.begin().expect("a writer begins");
        let r = db.begin_read(false).expect("a reader begins");
        drop_panicking(w);
        drop_panicking(r);
        let tx = db.begin().expect("a writer begins");
        db.commit(tx).expect("the writer commits");
        assert_eq!(nothing(), holding(db));
        assert_eq!(2, db.take_ends_by_drop());
    });
}

#[test]
fn a_commit_panicking_after_its_serializer_commit_releases_the_rest_on_unwinding() {
    at(7, |db| {
        let w = db.begin().expect("a writer begins");
        let x = w.xid().expect("a writer has an id");
        let r = db.begin_read(false).expect("a reader begins");
        db.fail_next_log_safe("the injected failure");
        let ran = catch_unwind(AssertUnwindSafe(|| db.commit(w)));
        let message = ran.err().and_then(|p| p.downcast_ref::<String>().cloned());
        assert_eq!(Some("the injected failure".to_owned()), message);
        assert_eq!((1, 1, BTreeSet::new(), BTreeSet::new()), holding(db));
        assert_eq!(Ok(Status::Committed), db.txns().recorded(db.pager(), x));
        assert_eq!(0, db.take_ends_by_drop());
        db.abort(r).expect("the reader ends");
        let tx = db.begin().expect("a writer begins");
        db.commit(tx).expect("the writer commits");
        assert_eq!(nothing(), holding(db));
    });
}

#[test]
fn an_abort_panicking_after_its_serializer_abort_releases_the_rest_on_unwinding() {
    at(8, |db| {
        let w = db.begin().expect("a writer begins");
        let x = w.xid().expect("a writer has an id");
        db.fail_next_log_safe("the injected failure");
        let ran = catch_unwind(AssertUnwindSafe(|| db.abort(w)));
        let message = ran.err().and_then(|p| p.downcast_ref::<String>().cloned());
        assert_eq!(Some("the injected failure".to_owned()), message);
        assert_eq!(nothing(), holding(db));
        assert_eq!(Ok(Status::Aborted), db.txns().recorded(db.pager(), x));
        assert_eq!(0, db.take_ends_by_drop());
        let tx = db.begin().expect("a writer begins");
        db.commit(tx).expect("the writer commits");
        assert_eq!(nothing(), holding(db));
    });
}
