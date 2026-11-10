use std::sync::Arc;
use std::task::Waker;

use adtb_sys::lock::Lock;
use adtb_sys::os::{Backend, OsRuntime};

use crate::exec::{Resumable, Step};

use super::super::text::Prepared;
use super::super::{CreateOptions, Database, Log, OpenOptions};
use super::call::{Call, Work};
use super::task::{Shared, State, TaskOptions};

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
