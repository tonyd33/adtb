use std::collections::BTreeSet;

use adtb_lang::ids::CatCtorId;
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::process::finished;

use crate::heap::Heap;
use crate::key::{OrderKey, Value};
use crate::mvcc::{Scope, Transactions, Xid};
use crate::runtime::Runtime;
use crate::storage::{Damage, Pager, Pid};
use crate::test_world::options;

use super::super::deployment::{Fact, Mark, Origin};
use super::super::ids::{CatConstraintId, CatFieldId, CatIndexId, CatRelId};
use super::super::kept::{KeptField, KeptRow};
use super::super::readers::Kind;
use super::super::roots;
use super::super::{Abandoning, Catalog, Error, ROOTS};
use super::{RootsRow, Rot};

/// Insert `row`, unadmitted, into the roots heap `roots` as transaction `x`.
fn insert_row<R: Runtime>(
    pager: &Pager<R>,
    txns: &Transactions<R>,
    roots: &Heap,
    x: Xid,
    row: &[Value],
) -> Result<(), Error> {
    let mut judge = txns.judge_now(pager, roots.id().0);
    roots
        .insert_pruning(pager, x, row, &mut judge)
        .kept(&Abandoning(()))?;
    Ok(())
}

fn int(n: i64) -> Value {
    Value::I64(n)
}

fn text(t: &str) -> Value {
    Value::Text(t.into())
}

fn sum(ctor: u32, fields: Vec<Value>) -> Value {
    Value::Sum { ctor, fields }
}

/// Returns a damaged row of each roots constructor other than `Kept` with a field that can hold a
/// bad value, with its refusal.
fn damaged() -> Vec<(Value, Rot)> {
    let held = |dropped: i64| vec![int(dropped), text("m"), text("n"), text("b")];
    let key_bytes = Value::Bytes(vec![1, 2, 3]);
    let held_key = [held(1), vec![int(5), int(6), key_bytes]].concat();
    vec![
        (
            sum(0, vec![int(1), int(0), int(3), int(-7)]),
            Rot::NegativeId(-7),
        ),
        (sum(1, vec![int(5), int(-7)]), Rot::NegativeId(-7)),
        (
            sum(2, vec![int(9), Value::Bytes(Vec::new())]),
            Rot::OrderKey(Vec::new()),
        ),
        (sum(3, vec![int(-2), text("")]), Rot::NegativeId(-2)),
        (
            sum(4, vec![int(1), int(0), int(1 << 40), int(7)]),
            Rot::HeapIdWide(1 << 40),
        ),
        (sum(5, vec![int(1), int(-4), int(7)]), Rot::NegativeId(-4)),
        (
            sum(6, [held(-1), vec![int(3)]].concat()),
            Rot::NegativeId(-1),
        ),
        (
            sum(7, [held(1), vec![int(-3)]].concat()),
            Rot::NegativeId(-3),
        ),
        (
            sum(8, [held(1), vec![int(-3)]].concat()),
            Rot::NegativeId(-3),
        ),
        (
            sum(9, held_key.clone()),
            Rot::PendingRow {
                ctor: 9,
                fields: held_key,
            },
        ),
        (sum(10, vec![int(-5), text("r")]), Rot::NegativeId(-5)),
        (
            sum(13, vec![int(1), int(2), int(2), text("t")]),
            Rot::Origin(2),
        ),
    ]
}

/// Returns, for each reader of the roots heap, its refusal of a catalog whose roots heap holds a
/// committed row `row` beside the rows the catalog wrote.
fn readers_of(seed: u64, row: &Value) -> Vec<(&'static str, Option<Error>)> {
    let mut out = Vec::new();
    let mut sim = Sim::new(seed, Config::default());
    finished(&mut sim, |rt: SimRuntime| {
        let pager = Pager::create(rt.clone(), &options(64)).expect("create");
        let (catalog, txns) = Catalog::create(rt.clone(), &pager).expect("the catalog");
        let commit = |txns: &Transactions<SimRuntime>| {
            let begun = txns.begin(&pager).expect("a transaction");
            insert_row(
                &pager,
                txns,
                catalog.roots.heap(),
                begun.xid(),
                std::slice::from_ref(row),
            )
            .expect("the row is written");
            txns.commit(&pager, begun).expect("the commit").done();
        };
        commit(&txns);
        out.push(("retained", catalog.retained(&pager, &txns).err()));
        out.push(("kept", catalog.kept(&pager, &txns).err()));
        out.push(("deployed", catalog.deployed(&pager, &txns).err()));
        let begun = txns.begin(&pager).expect("a transaction");
        let hold = txns.take(Scope::Every).expect("the hold");
        let w = super::super::Writer::new(&hold, begun.xid());
        let replaced = catalog.replace_kept(&pager, &txns, w, (339, "g"), &[]);
        drop(hold);
        txns.abort(&pager, begun).expect("the abort");
        out.push(("replace_kept", replaced.err()));
        drop(txns);
        out.push(("open", Catalog::open(rt, &pager).err()));
    });
    out
}

#[test]
fn every_reader_refuses_a_roots_row_holding_a_value_its_constructor_refuses() {
    let rows = damaged();
    assert_eq!(12, rows.len());
    let readers = ["retained", "kept", "deployed", "replace_kept", "open"];
    let mut want = Vec::new();
    let mut got = Vec::new();
    for (seed, (row, rot)) in (1..).zip(rows) {
        got.push((row.clone(), readers_of(seed, &row)));
        let refusals = readers
            .into_iter()
            .map(|r| (r, Some(Error::Corrupt(rot.clone()))))
            .collect();
        want.push((row, refusals));
    }
    let blob = sum(
        14,
        vec![
            int(339),
            text("g"),
            int(0),
            int(1),
            Value::Bytes(Vec::new()),
        ],
    );
    got.push((blob.clone(), readers_of(20, &blob)));
    let refusals = readers
        .into_iter()
        .map(|r| {
            (
                r,
                (r == "kept").then_some(Error::Corrupt(Rot::KeptFormat(1))),
            )
        })
        .collect();
    want.push((blob, refusals));
    assert_eq!(want, got);
}

fn kept_sample(ctor: Option<CatCtorId>) -> KeptRow {
    KeptRow {
        relation: 339,
        group: "database".into(),
        ctor,
        fields: vec![
            KeptField::Int(7),
            KeptField::Instant(-3),
            KeptField::Text("page 9".into()),
        ],
    }
}

/// Returns a row of each constructor of the roots heap, and of each case within one.
fn samples() -> Vec<RootsRow> {
    let held = |kind| RootsRow::Held {
        dropped: 4,
        module: "app".into(),
        name: "c".into(),
        holder: "app-2".into(),
        kind,
    };
    let kept = |ctor| roots::kept(&kept_sample(ctor));
    let mark = |origin| Mark {
        role: -2,
        origin,
        text: "bundle".into(),
    };
    vec![
        RootsRow::HeapRoot {
            rel: CatRelId::from_raw(11),
            ctor: CatCtorId::from_raw(12),
            heap: u32::MAX,
            at: Pid(13),
        },
        RootsRow::IndexRoot {
            index: 1 << 40,
            at: Pid(14),
        },
        RootsRow::OrderKey {
            ctor: 15,
            key: OrderKey::new(vec![0x80, 0x02]).expect("an order key"),
        },
        RootsRow::Migration {
            version: 16,
            text: "migration 16".into(),
        },
        RootsRow::HistoryHeap {
            rel: CatRelId::from_raw(17),
            ctor: CatCtorId::from_raw(18),
            heap: 19,
            at: Pid(20),
        },
        RootsRow::HistoryIndex {
            rel: CatRelId::from_raw(21),
            owner: 22,
            at: Pid(23),
        },
        held(Kind::Constraint(CatConstraintId::from_raw(24))),
        held(Kind::Check(CatConstraintId::from_raw(25))),
        held(Kind::Unique(CatIndexId::from_raw(26))),
        held(Kind::Key {
            index: CatIndexId::from_raw(27),
            relation: CatRelId::from_raw(28),
            fields: vec![CatFieldId::from_raw(29), CatFieldId::from_raw(1 << 50)],
        }),
        RootsRow::Dormant {
            version: 30,
            role: "reader".into(),
        },
        RootsRow::Deployment(Fact::Deployed),
        RootsRow::Deployment(Fact::Family("app".into(), -31)),
        RootsRow::Deployment(Fact::Registered(32, mark(Origin::Open))),
        RootsRow::Deployment(Fact::Registered(33, mark(Origin::Command))),
        kept(None),
        kept(Some(CatCtorId::from_raw(318))),
        RootsRow::Deployment(Fact::Expired(-34, 35)),
    ]
}

#[test]
fn a_row_of_every_roots_constructor_reads_back_through_the_roots_heap() {
    let rows = samples();
    let ids: BTreeSet<u32> = rows.iter().map(|r| r.parts().0).collect();
    assert_eq!((0..16).collect::<BTreeSet<u32>>(), ids);
    let mut read = Vec::new();
    let mut sim = Sim::new(5, Config::default());
    finished(&mut sim, |rt: SimRuntime| {
        let pager = Pager::create(rt.clone(), &options(64)).expect("create");
        let (catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        let begun = txns.begin(&pager).expect("a transaction");
        for row in &rows {
            insert_row(
                &pager,
                &txns,
                catalog.roots.heap(),
                begun.xid(),
                &row.encode(),
            )
            .expect("the row is written");
        }
        txns.commit(&pager, begun).expect("the commit").done();
        let hold = txns
            .take(Scope::Heaps(BTreeSet::from([ROOTS])))
            .expect("a snapshot");
        read = (catalog
            .roots
            .rows(&pager, &txns, (&hold, None))
            .expect("the roots rows"))
        .into_iter()
        .map(|(_, _, row)| row)
        .collect();
    });
    let missing: Vec<&RootsRow> = rows.iter().filter(|r| !read.contains(r)).collect();
    assert_eq!(Vec::<&RootsRow>::new(), missing);
}

/// Returns the version that named each heap of a catalog after one migration, then its refusal
/// of a heap named by a later transaction that applied no migration.
fn born_after(seed: u64) -> (BTreeSet<u64>, Option<Error>) {
    let mut out = None;
    let mut sim = Sim::new(seed, Config::default());
    finished(&mut sim, |rt: SimRuntime| {
        let pager = Pager::create(rt.clone(), &options(64)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        catalog
            .migrate(
                &pager,
                &txns,
                "module m\nadd module m\nadd type T = { id: I64 }\nadd relation ts : T key id\n",
            )
            .expect("the migration applies");
        let retained = catalog.retained(&pager, &txns).expect("the history");
        let versions = retained.born.values().copied().collect();
        let begun = txns.begin(&pager).expect("a transaction");
        let stray = RootsRow::HeapRoot {
            rel: CatRelId::from_raw(1 << 20),
            ctor: CatCtorId::from_raw(1 << 20),
            heap: 4242,
            at: Pid(7),
        };
        insert_row(
            &pager,
            &txns,
            catalog.roots.heap(),
            begun.xid(),
            &stray.encode(),
        )
        .expect("the row is written");
        txns.commit(&pager, begun).expect("the commit").done();
        out = Some((versions, catalog.retained(&pager, &txns).err()));
    });
    out.expect("the case ran")
}

#[test]
fn a_heap_no_roots_row_names_is_damage_to_the_history() {
    let mut out = None;
    let mut sim = Sim::new(12, Config::default());
    finished(&mut sim, |rt: SimRuntime| {
        let pager = Pager::create(rt.clone(), &options(64)).expect("create");
        let (catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        let roots = catalog.roots.heap();
        let mut versions = Vec::new();
        roots
            .scan(&pager, |v| versions.push(v))
            .expect("the roots scan");
        let (v, heap) = versions
            .into_iter()
            .find_map(|v| match RootsRow::decode(&v.row) {
                Ok(RootsRow::HeapRoot { heap, .. }) => Some((v, heap)),
                _ => None,
            })
            .expect("the create names a heap");
        let begun = txns.begin(&pager).expect("a transaction");
        roots
            .delete(
                &pager,
                crate::heap::WriteTarget::checked(v.tid, v.header),
                begun.xid(),
            )
            .kept(&Abandoning(()))
            .expect("the row is deleted")
            .unchecked(&Abandoning(()));
        txns.commit(&pager, begun).expect("the commit").done();
        out = Some((heap, catalog.retained(&pager, &txns).err()));
    });
    let (heap, refused) = out.expect("the case ran");
    assert_eq!(Some(Error::Corrupt(Rot::Nameless(heap))), refused);
}

#[test]
fn every_heap_is_born_at_the_create_or_a_migration_and_one_named_by_neither_is_damage() {
    let (versions, refused) = born_after(11);
    assert_eq!(BTreeSet::from([0, 1]), versions);
    assert_eq!(Some(Error::Corrupt(Rot::Unborn(4242))), refused);
}

#[test]
fn a_roots_row_of_another_shape_is_refused_whole() {
    let arity = [4, 2, 2, 2, 4, 3, 5, 5, 5, 7, 2, 0, 2, 4, 5, 2, 0];
    let refused = |ctor: u32, fields: Vec<Value>| match ctor {
        6..=9 => Rot::PendingRow { ctor, fields },
        11..=13 | 15 => Rot::DeploymentRow { ctor, fields },
        14 => Rot::KeptRow(fields),
        _ => Rot::RootsCtor { ctor, fields },
    };
    let mut want = Vec::new();
    let mut got = Vec::new();
    for (ctor, n) in (0..).zip(arity) {
        let lengths = if n == 0 {
            vec![n + 1]
        } else {
            vec![n - 1, n + 1]
        };
        for len in lengths {
            let fields = vec![int(0); len];
            got.push(RootsRow::decode(&[sum(ctor, fields.clone())]));
            want.push(Err(refused(ctor, fields)));
        }
    }
    assert_eq!(want, got);
    assert_eq!(32, got.len());
}

#[test]
fn a_roots_row_that_is_not_one_sum_is_refused_whole() {
    let one = sum(11, Vec::new());
    for row in [
        Vec::new(),
        vec![int(1)],
        vec![one.clone(), one],
        vec![Value::Record(Vec::new())],
    ] {
        assert_eq!(Err(Rot::RootsRow(row.clone())), RootsRow::decode(&row));
    }
}

#[test]
fn a_roots_row_holding_a_value_its_constructor_refuses_is_refused_whole() {
    let mut rows = damaged();
    rows.push((sum(3, vec![text("1"), text("")]), Rot::NotAnId(text("1"))));
    let want: Vec<_> = rows.iter().map(|(_, rot)| Err(rot.clone())).collect();
    let got: Vec<_> = rows
        .iter()
        .map(|(row, _)| RootsRow::decode(std::slice::from_ref(row)))
        .collect();
    assert_eq!(want, got);
}

#[test]
fn a_kept_row_holding_a_value_it_refuses_is_refused_whole() {
    let fields = |relation, format, bytes: Vec<u8>| {
        vec![
            int(relation),
            text("g"),
            int(318),
            int(format),
            Value::Bytes(bytes),
        ]
    };
    let cases = [
        (fields(339, 3, Vec::new()), Rot::KeptFormat(3)),
        (fields(339, 1, Vec::new()), Rot::KeptFormat(1)),
        (fields(-339, 2, Vec::new()), Rot::NegativeId(-339)),
        (
            fields(339, 2, vec![9]),
            Rot::KeptRow(fields(339, 2, vec![9])),
        ),
    ];
    let mut want = Vec::new();
    let mut got = Vec::new();
    for (stored, rot) in cases {
        let [
            Value::I64(relation),
            Value::Text(group),
            Value::I64(ctor),
            Value::I64(format),
            Value::Bytes(row),
        ] = stored.as_slice()
        else {
            panic!("a kept row's fields");
        };
        got.push(roots::kept_row(*relation, group, *ctor, *format, row));
        want.push(Err(rot));
    }
    assert_eq!(want, got);
}

#[test]
fn a_kept_row_reads_back_its_constructor_and_fields() {
    for ctor in [None, Some(CatCtorId::from_raw(318))] {
        let RootsRow::Kept {
            relation,
            group,
            ctor: c,
            format,
            row,
        } = roots::kept(&kept_sample(ctor))
        else {
            panic!("a kept row");
        };
        assert_eq!(
            Ok(kept_sample(ctor)),
            roots::kept_row(relation, &group, c, format, &row)
        );
    }
}

#[test]
fn every_roots_row_encodes_as_the_file_format_holds_it() {
    let held =
        |tail: Vec<Value>| [vec![int(4), text("app"), text("c"), text("app-2")], tail].concat();
    let key: Vec<u8> = [29u64.to_le_bytes(), (1u64 << 50).to_le_bytes()].concat();
    let page = |ctor| {
        let mut fields = vec![0];
        fields.extend(7i64.to_le_bytes());
        fields.push(2);
        fields.extend((-3i64).to_le_bytes());
        fields.push(1);
        fields.extend(6u64.to_le_bytes());
        fields.extend(b"page 9");
        vec![
            int(339),
            text("database"),
            int(ctor),
            int(2),
            Value::Bytes(fields),
        ]
    };
    let want = vec![
        sum(0, vec![int(11), int(12), int(4_294_967_295), int(13)]),
        sum(1, vec![int(1 << 40), int(14)]),
        sum(2, vec![int(15), Value::Bytes(vec![0x80, 0x02])]),
        sum(3, vec![int(16), text("migration 16")]),
        sum(4, vec![int(17), int(18), int(19), int(20)]),
        sum(5, vec![int(21), int(22), int(23)]),
        sum(6, held(vec![int(24)])),
        sum(7, held(vec![int(25)])),
        sum(8, held(vec![int(26)])),
        sum(9, held(vec![int(27), int(28), Value::Bytes(key)])),
        sum(10, vec![int(30), text("reader")]),
        sum(11, Vec::new()),
        sum(12, vec![text("app"), int(-31)]),
        sum(13, vec![int(32), int(-2), int(0), text("bundle")]),
        sum(13, vec![int(33), int(-2), int(1), text("bundle")]),
        sum(14, page(0)),
        sum(14, page(318)),
        sum(15, vec![int(-34), int(35)]),
    ];
    let got: Vec<Value> = samples().iter().flat_map(RootsRow::encode).collect();
    assert_eq!(want, got);
}

#[test]
fn a_catalog_opens_over_a_kept_row_whose_fields_do_not_decode() {
    let blob = sum(
        14,
        vec![int(339), text("g"), int(0), int(2), Value::Bytes(vec![9])],
    );
    let mut status = None;
    let mut sim = Sim::new(21, Config::default());
    finished(&mut sim, |rt: SimRuntime| {
        let pager = Pager::create(rt.clone(), &options(64)).expect("create");
        let (catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        status = txns.commit_log().status_pages().values().next().copied();
        let begun = txns.begin(&pager).expect("a transaction");
        insert_row(
            &pager,
            &txns,
            catalog.roots.heap(),
            begun.xid(),
            std::slice::from_ref(&blob),
        )
        .expect("the row is written");
        txns.commit(&pager, begun).expect("the commit").done();
        pager.flush_log();
    });
    let status = status.expect("a status page");
    let files = sim.files();
    let mut got = Vec::new();
    for damaged in [false, true] {
        let mut sim = Sim::with_files(22, Config::default(), files.clone());
        finished(&mut sim, |rt: SimRuntime| {
            let (pager, _) = Pager::open(rt.clone(), &options(64)).expect("reopen");
            if damaged {
                pager.plant_damage(status, Damage::Checksum);
            }
            let opened = Catalog::open(rt, &pager);
            let kept = opened.as_ref().map(|(catalog, txns)| {
                (!damaged)
                    .then(|| catalog.kept(&pager, txns).err())
                    .flatten()
            });
            got.push((
                damaged,
                pager.whole_cause().is_some(),
                kept.map_err(Clone::clone),
            ));
        });
    }
    let refused = Some(Error::Corrupt(Rot::KeptRow(vec![
        int(339),
        text("g"),
        int(0),
        int(2),
        Value::Bytes(vec![9]),
    ])));
    assert_eq!(
        vec![(false, false, Ok(refused)), (true, true, Ok(None))],
        got
    );
}
