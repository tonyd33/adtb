use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};

use adtb_sim::{Config, Outcome, SimRuntime};
use adtb_sys::lock::Ranked;
use adtb_testkit::fixture::Fixture;

use crate::mvcc::{Gate, Status, Xid};
use crate::ranks::{self, broken, order_broken};
use crate::test_world::write_path::{self, WritePath};

use super::Database;

const ITEMS: &str =
    "type Item = { id: Key items, n: I64 }\nrelation items : Item key id generate sequence\n";

/// What a database's transactions hold: the serializer's entries, the readers' snapshots, the
/// ids the waits hold running and the ids the transactions hold running.
type Holding = (usize, usize, BTreeSet<Xid>, BTreeSet<Xid>);

fn holding(db: &Database<SimRuntime>) -> Holding {
    let gates = [Gate::Offline.xid(), Gate::VacuumHolder.xid()];
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

/// Drop `tx` on a thread panicking with the injected failure.
fn drop_panicking<T: Send>(tx: T) {
    let ran = catch_unwind(AssertUnwindSafe(move || {
        let _tx = tx;
        panic!("the injected failure");
    }));
    let message = ran.err().and_then(|p| p.downcast_ref::<&str>().copied());
    assert_eq!(Some("the injected failure"), message);
}

/// A drop on a panicking thread or after the pager closes releases what the transaction holds,
/// records nothing, and counts its abort unrecorded and its end by drop.
#[test]
fn a_drop_while_panicking_or_once_closed_releases_records_nothing_and_counts() {
    at(3, |db| {
        let w = db.begin().expect("a writer begins");
        let x = w.xid().expect("a writer has an id");
        let r = db.begin_read(false).expect("a reader begins");
        drop_panicking(w);
        drop_panicking(r);
        assert_eq!(nothing(), holding(db));
        assert_eq!(Ok(Status::InProgress), db.txns().recorded(db.pager(), x));
        assert_eq!(1, db.txns().unrecorded_aborts());
        assert_eq!(2, db.take_ends_by_drop());
        let w = db.begin().expect("a writer begins");
        let r = db.begin_read(false).expect("a reader begins");
        db.pager().close();
        let files = db.pager().rt().files();
        drop(w);
        drop(r);
        assert_eq!(nothing(), holding(db));
        assert_eq!(files, db.pager().rt().files());
        assert_eq!(2, db.txns().unrecorded_aborts());
        assert_eq!(2, db.take_ends_by_drop());
    });
}

/// A catalog transaction ends through the one end user transactions take: once it commits,
/// its id is recorded committed and out of the running set and the waits, and a user writer
/// running beside it holds what it held.
#[test]
fn a_catalog_end_leaves_only_the_user_writer_beside_it_held() {
    use crate::catalog::Rewrite;
    use crate::catalog::codec::{Codec, ROLE_CREATED, ROLE_ID, ROLE_NAME, RoleRow};
    use crate::key::Value as Field;
    use adtb_lang::ids::engine_ctor as k;

    let f = Fixture::new(ITEMS);
    let mut sim = write_path::sim(4, Config::default(), &f);
    let outcome = sim.run(|rt| {
        let w = WritePath::loaded(&rt, &f, None);
        let db = w.db();
        let user = db.begin().expect("a writer begins");
        let y = user.xid().expect("a writer has an id");
        let x = db.txns().next();
        let codec = Codec::<RoleRow>::new(w.catalog().schema());
        let row = codec.encode(
            k::ROLE,
            [
                (ROLE_ID, Field::I64(100)),
                (ROLE_NAME, Field::Text("r100".to_owned())),
                (ROLE_CREATED, Field::Instant(0)),
            ],
        );
        let rewrite = Rewrite {
            relation: codec.relation(),
            deleted: Vec::new(),
            inserted: vec![row],
        };
        (w.catalog())
            .deploy(db.pager(), db.txns(), &[rewrite])
            .expect("the catalog commits");
        assert_eq!(Ok(Status::Committed), db.txns().recorded(db.pager(), x));
        let ys = BTreeSet::from([y]);
        assert_eq!((1, 0, ys.clone(), ys), holding(db));
        db.abort(user).expect("the writer aborts");
        assert_eq!(nothing(), holding(db));
    });
    assert_eq!(Outcome::Finished, outcome);
}

#[test]
fn the_vacuum_lock_waited_for_under_the_bundle_writer_panics() {
    at(9, |db| {
        let _writer = ranks::BUNDLE_WRITER.hold();
        assert_eq!(
            broken("the vacuum lock", "the bundle writer lock"),
            order_broken(|| drop(db.holding_off_vacuum()))
        );
    });
}

#[test]
fn an_index_latch_waited_for_under_another_or_the_serializer_panics() {
    at(10, |db| {
        let latch = db
            .latches
            .values()
            .next()
            .expect("the items have a key index");
        let ser = db.session.ser.lock(&db.rt);
        assert_eq!(
            broken("an index latch", "the serializer lock"),
            order_broken(|| drop(latch.lock(&db.rt)))
        );
        drop(ser);
        let _held = latch.lock(&db.rt);
        let other = Ranked::new(ranks::INDEX_LATCH, ());
        assert_eq!(
            broken("an index latch", "an index latch"),
            order_broken(|| drop(other.lock(&db.rt)))
        );
    });
}

#[test]
fn the_serializer_waited_for_under_the_readers_panics() {
    at(11, |db| {
        let _readers = db.session.readers.lock(&db.rt);
        assert_eq!(
            broken("the serializer lock", "the readers lock"),
            order_broken(|| drop(db.session.ser.lock(&db.rt)))
        );
    });
}

#[test]
fn the_readers_waited_for_under_the_waits_panics() {
    at(12, |db| {
        let _waits = ranks::WAITS.hold();
        assert_eq!(
            broken("the readers lock", "the waits lock"),
            order_broken(|| drop(db.session.readers.lock(&db.rt)))
        );
    });
}

#[test]
fn the_waits_waited_for_under_the_dead_counts_panics() {
    at(13, |db| {
        let _dead = db.session.dead.lock(&db.rt);
        assert_eq!(
            broken("the waits lock", "the dead counts lock"),
            order_broken(|| drop(db.session.waits().running()))
        );
    });
}

#[test]
fn the_dead_counts_waited_for_under_the_sequences_panics() {
    at(14, |db| {
        let _sequences = db.sequences.lock(&db.rt);
        assert_eq!(
            broken("the dead counts lock", "the sequences lock"),
            order_broken(|| drop(db.session.dead.lock(&db.rt)))
        );
    });
}

#[test]
fn the_sequences_waited_for_under_the_task_queue_panics() {
    at(15, |db| {
        let _queue = ranks::TASK_QUEUE.hold();
        assert_eq!(
            broken("the sequences lock", "the task queue lock"),
            order_broken(|| drop(db.sequences.lock(&db.rt)))
        );
    });
}

#[test]
fn damage_mapped_under_the_serializer_panics() {
    at(16, |db| {
        let _ser = db.session.ser.lock(&db.rt);
        assert_eq!(
            "damage is mapped under the serialization lock",
            order_broken(|| drop(db.fail(super::Broken::ReadOnly.into())))
        );
    });
}

#[test]
fn a_verify_begun_while_the_thread_runs_a_writer_panics() {
    at(17, |db| {
        let _writing = super::Writing::enter();
        assert_eq!(
            "this thread waits on writers while it runs a transaction with an id",
            order_broken(|| drop(db.begin_verify()))
        );
    });
}
