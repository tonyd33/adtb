use std::sync::Arc;
use std::task::Waker;

use adtb_sys::lock::Lock;
use adtb_sys::os::{Backend, OsRuntime};

use crate::exec::{Resumable, Step};

use adtb_sim::SimRuntime;

use super::super::text::Prepared;
use super::super::{
    AdHocTransaction, CallOptions, CreateOptions, Database, Error, FatalReason, Log, OpenOptions,
    TxnCallOptions, TxnOptions,
};
use super::call::{Call, Work};
use super::result::Completion;
use super::task::{Request, Shared, State, TaskOptions};
use crate::exec::Stopped;
use crate::storage::{Damage, PageError};
use crate::test_world::load;
use crate::testing::Plant;

const APP: &str = "module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
";

#[test]
fn a_call_dropped_holding_its_read_leaves_nothing_of_its_transaction() {
    let scratch = adtb_testkit::scratch::Scratch::new("call-drop-fixtures", "call-drop");
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(1),
        pool_bytes: Some(4 * 1024 * 1024),
        ..OpenOptions::default()
    };
    let db = Database::open_in(rt, "drop.adt", &Log::new([APP]), &options).expect("open");
    let e = db.engine.clone();
    let caller = e.principal(None).expect("principal");
    let items = Prepared::parsed("from u in users select u").expect("parses");
    let p = Prepared::of_items(&e.schema, &e.program, &e.explained(), &items, &[], caller)
        .expect("checks");
    let shared = Arc::new(Shared {
        rt: e.rt.clone(),
        state: Lock::new(State::default()),
    });
    let work = Work::Text(Box::new(p));
    let mut call = Call::new(&e, shared, work, None, TaskOptions::default());
    assert_eq!(
        Step::Wait,
        call.run(Waker::noop()),
        "the output waits for room"
    );
    let held = |e: &super::super::Engine<OsRuntime>| (e.db.tracked(), e.db.readers_held());
    assert_eq!((1, 1), held(&e), "the call holds its read");
    drop(call);
    assert_eq!((0, 0), held(&e), "serializer, readers");
    assert_eq!(
        1,
        e.db.take_ends_by_drop(),
        "the call's transaction ended by drop"
    );
    drop(e);
    db.close().unwrap();
}

#[test]
fn a_close_after_a_transaction_ended_by_drop_panics() {
    let scratch = adtb_testkit::scratch::Scratch::new("call-drop-fixtures", "close-drop");
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(1),
        ..OpenOptions::default()
    };
    let db = Database::open_in(rt, "drop.adt", &Log::new([APP]), &options).expect("open");
    drop(db.engine.db.begin_read(false).expect("a reader begins"));
    let closed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| db.close()));
    let message = closed
        .err()
        .and_then(|p| p.downcast_ref::<String>().cloned());
    let expected = "assertion `left == right` failed: every transaction ended before the close \
                    was committed or aborted, none dropped\n  left: 0\n right: 1";
    assert_eq!(Some(expected), message.as_deref());
}

const RENAME: &str = r#"update u in users where u.name == "a" set User { u with name: "b" }"#;

/// What a write call left: its completion and how many more abort records went unrecorded.
type Left = (Option<Completion>, usize);

/// What a test does to a write call waiting on another transaction.
struct Waiting<'a> {
    call: Box<Call<SimRuntime>>,
    shared: Arc<Shared<SimRuntime>>,
    other: AdHocTransaction<'a, SimRuntime>,
    /// Refuse the status page of the call's id as unreadable.
    plant: &'a dyn Fn(),
    /// How an abort record fails on that page.
    failed: Error,
}

/// Run a write call of `RENAME`, then `fail "no"` as a second statement, until it waits on
/// another transaction renaming the same user, then run `then`. Returns what the call left and
/// how an abort record fails on the page `then` may refuse.
fn waiting_write(then: impl FnOnce(Waiting<'_>)) -> (Left, Error) {
    let mut sim = adtb_sim::Sim::new(6, adtb_sim::Config::default());
    adtb_harness::process::run(&mut sim, |rt| {
        let db = load::opened(
            &rt,
            crate::test_world::NAME,
            &Log::new([APP]),
            &load::options(2),
        );
        db.query_text(
            r#"insert users { name: "a" }"#,
            &[],
            &CallOptions::default(),
        )
        .expect("the insert");
        let mut other = db.begin(&TxnOptions::default()).expect("a transaction");
        other
            .query_text(RENAME, &[], &TxnCallOptions::default())
            .expect("the other rename");
        let e = db.engine.clone();
        let caller = e.principal(None).expect("principal");
        let text = format!("{RENAME}\nfail \"no\"");
        let items = Prepared::parsed(&text).expect("parses");
        let p = Prepared::of_items(&e.schema, &e.program, &e.explained(), &items, &[], caller)
            .expect("checks");
        let shared = Arc::new(Shared {
            rt: e.rt.clone(),
            state: Lock::new(State::default()),
        });
        let work = Work::Text(Box::new(p));
        let call = Call::new(&e, shared.clone(), work, None, TaskOptions::default());
        let mut call = Box::new(call);
        let waits = call.run(Waker::noop());
        assert_eq!(Step::Wait, waits, "the call waits on the other");
        let pages = db.testing().commit_log().status().clone();
        let page = *pages.values().next_back().expect("a status page");
        let plant = || {
            let refused = Plant::Refuse {
                page,
                damage: Damage::Unreadable,
            };
            db.testing().plant(refused).expect("planted");
        };
        let unreadable = PageError {
            pid: page,
            damage: Damage::Unreadable,
        };
        let failed = Error::Fatal {
            reason: FatalReason::Storage {
                detail: format!("{:?}", crate::mvcc::Error::Corrupt(unreadable)),
            },
        };
        let unrecorded = e.db.txns().unrecorded_aborts();
        then(Waiting {
            call,
            shared: shared.clone(),
            other,
            plant: &plant,
            failed: failed.clone(),
        });
        let settled = shared.state.lock(&e.rt).settled.clone();
        let counted = e.db.txns().unrecorded_aborts() - unrecorded;
        drop(e);
        db.close().expect("the database closes");
        ((settled, counted), failed)
    })
}

#[test]
fn a_write_call_stopped_while_its_abort_record_fails_settles_the_records_failure() {
    let (left, failed) = waiting_write(|mut w| {
        w.shared.state.lock(&w.shared.rt).request = Some(Request::Cancel);
        w.other.rollback().expect("the other rolls back");
        (w.plant)();
        assert_eq!(Step::Done, w.call.run(Waker::noop()), "the call fails");
    });
    assert_eq!((Some(Completion::Failed(failed)), 1), left);
}

#[test]
fn a_write_call_released_while_its_abort_record_fails_settles_the_records_failure() {
    let (left, failed) = waiting_write(|w| {
        (w.plant)();
        w.call.stop(Stopped::Cancelled);
        assert_eq!(
            Err(w.failed),
            w.other.rollback(),
            "the other's abort record fails"
        );
    });
    assert_eq!((Some(Completion::Failed(failed)), 1), left);
}
