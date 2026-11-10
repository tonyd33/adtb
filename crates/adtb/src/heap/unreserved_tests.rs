//! A version or chunk whose header names a writer past the commit log's reservation: each reader
//! or writer that looks the writer up refuses the page holding it as damage.

use std::collections::BTreeMap;

use adtb_harness::process;
use adtb_sim::SimRuntime;

use super::{Error as HeapError, FieldType, Heap, LargeStore, Refused};
use crate::key::{Scalar, Value};
use crate::mvcc::{StatusError, Transactions, Writers, Xid};
use crate::storage::{Damage, Options, PageError, Pager, Pid, Shape, Walk};
use crate::test_world::api;
use crate::test_world::load as open;
use crate::testing::Plant;
use crate::{Database, Error, FatalReason};

const APP: &str = "type User = { id: Key users, name: Text }
relation users : User key id generate sequence
";

const PAST: Xid = Xid(1 << 40);

const FRAME: Damage = Damage::Shape(Shape::Frame);

/// Insert one user into a new database and make its version's deleter [`PAST`] in place.
/// Returns the database and the page holding the version.
fn with_unreserved_deleter(rt: &SimRuntime) -> (Database<SimRuntime>, Pid) {
    let db = open::open(rt, APP);
    let t = db.testing();
    let first = t.next_xid();
    api::results(&db, r#"insert users { id: users.key(1), name: "a" }"#);
    let page = t.pages(t.heap("app::users", "User").unwrap()).unwrap()[0];
    let bytes = t.probe().page(page).unwrap();
    let headers: Vec<(usize, Xid)> = (first.0..t.next_xid().0)
        .map(Xid)
        .flat_map(|x| {
            let w = Writers::inserted(x).encode();
            let bytes = &bytes;
            (0..=bytes.len() - Writers::LEN)
                .filter(move |&i| bytes[i..i + Writers::LEN] == w)
                .map(move |i| (i, x))
        })
        .collect();
    assert_eq!(1, headers.len(), "one version's writers: {headers:?}");
    let (at, x) = headers[0];
    t.plant(Plant::Write {
        page,
        at,
        bytes: Writers::inserted(x).superseded_by(PAST).encode().to_vec(),
    })
    .unwrap();
    (db, page)
}

/// Run `text` over a version naming an unreserved deleter and check that it fails as the users
/// relation quarantined, with the version's page recorded as out of frame.
fn refused(seed: u64, text: &str) {
    process::once(seed, |rt| {
        let (db, page) = with_unreserved_deleter(&rt);
        let got = api::failure(&db, text);
        assert!(
            matches!(
                &got,
                Error::Fatal {
                    reason: FatalReason::Quarantined { what }
                } if what == "app::users.User"
            ),
            "{text}: {got:?}"
        );
        let refusals = db.testing().probe().refusals();
        assert_eq!(BTreeMap::from([(page, FRAME)]), refusals, "{text}");
        db.close().unwrap();
    });
}

#[test]
fn a_vacuum_meeting_an_unreserved_deleter_refuses_its_page() {
    refused(1, "vacuum users");
}

#[test]
fn a_read_meeting_an_unreserved_deleter_refuses_its_page() {
    refused(2, "from u in users select u.name");
}

#[test]
fn an_update_reading_an_unreserved_deleter_refuses_its_page() {
    refused(
        3,
        r#"update u in users where key u == users.key(1) set { u with name: "b" }"#,
    );
}

#[test]
fn a_unique_probe_meeting_an_unreserved_deleter_refuses_its_page() {
    refused(4, r#"insert users { id: users.key(1), name: "b" }"#);
}

fn options() -> Options {
    Options {
        page_size: 4096,
        pool_frames: 32,
        ..Options::new("unreserved.adt")
    }
}

fn refused_page(pid: Pid) -> PageError {
    PageError { pid, damage: FRAME }
}

#[test]
fn an_update_pruning_a_full_page_refuses_it_at_an_unreserved_deleter() {
    process::once(5, |rt| {
        let pager = Pager::create(rt.clone(), &options()).unwrap();
        let txns = Transactions::create(rt, &pager).unwrap();
        let large = LargeStore::create(&pager, 40).unwrap();
        let heap = Heap::create(&pager, 41, vec![FieldType::Scalar(Scalar::I64)], large).unwrap();
        let x = txns.begin(&pager).unwrap();
        let dead = heap.insert(&pager, x, &[Value::I64(0)]).unwrap();
        let target = heap.insert(&pager, x, &[Value::I64(1)]).unwrap();
        while heap.pages(&pager).unwrap().len() == 1 {
            heap.insert(&pager, x, &[Value::I64(2)]).unwrap();
        }
        txns.commit(&pager, x).unwrap();
        let page = Pid(dead.page());
        assert_eq!(page, Pid(target.page()), "both versions on the full page");
        heap.delete(&pager, dead, PAST).unwrap();
        let y = txns.begin(&pager).unwrap();
        let mut judge = txns.judge_now(&pager, heap.id().0);
        let got = heap.update_pruning(&pager, target, y, &[Value::I64(3)], &[], &mut judge);
        assert_eq!(Err(HeapError::Page(refused_page(page))), got);
        assert_eq!(BTreeMap::from([(page, FRAME)]), pager.damaged());
        assert_eq!(Err(refused_page(page)), pager.read(page).map(drop));
        pager.close();
    });
}

#[test]
fn a_status_lookup_of_a_version_whose_header_changed_before_the_latch_reads_again() {
    process::once(7, |rt| {
        let pager = Pager::create(rt, &options()).unwrap();
        let large = LargeStore::create(&pager, 40).unwrap();
        let heap = Heap::create(&pager, 41, vec![FieldType::Scalar(Scalar::I64)], large).unwrap();
        let tid = heap.insert(&pager, Xid(1), &[Value::I64(1)]).unwrap();
        heap.insert(&pager, Xid(1), &[Value::I64(2)]).unwrap();
        let mut versions = Vec::new();
        heap.scan(&pager, |v| versions.push(v)).unwrap();
        let [changed, kept] = versions.as_slice() else {
            panic!("two versions: {versions:?}");
        };
        heap.delete(&pager, tid, PAST).unwrap();
        let unreserved = || Err::<(), _>(StatusError::Unreserved(PAST));
        assert_eq!(Ok(None), changed.looked_up(&pager, unreserved()));
        assert_eq!(BTreeMap::new(), pager.damaged());
        let page = Pid(tid.page());
        assert_eq!(
            Err(refused_page(page)),
            kept.looked_up(&pager, unreserved())
        );
        assert_eq!(BTreeMap::from([(page, FRAME)]), pager.damaged());
        pager.close();
    });
}

#[test]
fn a_sweep_meeting_a_chunk_of_an_unreserved_writer_refuses_its_page() {
    process::once(6, |rt| {
        let pager = Pager::create(rt.clone(), &options()).unwrap();
        let txns = Transactions::create(rt, &pager).unwrap();
        let large = LargeStore::create(&pager, 40).unwrap();
        let fields = vec![
            FieldType::Scalar(Scalar::I64),
            FieldType::Scalar(Scalar::Bytes),
        ];
        let heap = Heap::create(&pager, 41, fields, large).unwrap();
        let row = [Value::I64(1), Value::Bytes(vec![7; 20_000])];
        heap.insert(&pager, PAST, &row).unwrap();
        let head = heap.out_of_line(&pager).unwrap().unwrap()[0].first;
        large.recount(&pager, head, 0).unwrap();
        let page = Pid(head.page());
        let resolved = |x| txns.resolved(&pager, x);
        let got = large.sweep(&pager, resolved, &mut || true);
        assert_eq!(Err::<Walk, _>(Refused::Page(refused_page(page))), got);
        assert_eq!(BTreeMap::from([(page, FRAME)]), pager.damaged());
        pager.close();
    });
}
