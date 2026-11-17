use std::collections::BTreeSet;

use crate::heap::page::{HeapPage, Slot};
use crate::heap::{CtorType, Error, FieldType, HEADER_LEN, Heap, LargeStore};
use crate::key::{CtorId, Scalar, Value};
use crate::mvcc::Transactions;
use crate::storage::{Damage, PageError, Pager, Pid, Shape, Undecoded};
use crate::verify::{self, Finding, Inventory, Problem, Report, Sum};
use adtb_harness::process::finished;
use adtb_sim::{Config, Sim};

use crate::test_world::options;

const FRAMES: usize = 32;

/// One stored row kind, the row that holds it, and how to break the byte that decides it.
struct Case {
    kind: &'static str,
    row: Vec<FieldType>,
    value: Vec<Value>,
    /// The bytes found once in the row's version, or in its value's first chunk with `store`,
    /// and the edit to the page relative to where they start.
    marker: Vec<u8>,
    edit: fn(&mut [u8], usize),
    store: bool,
    cause: Undecoded,
}

fn scalar(s: Scalar) -> FieldType {
    FieldType::Scalar(s)
}

fn ctor(id: u32, fields: Vec<FieldType>) -> CtorType {
    CtorType {
        id: CtorId(id),
        fields,
    }
}

fn sum(ctor: u32, fields: Vec<Value>) -> Value {
    Value::Sum { ctor, fields }
}

/// A sum split between the fixed part, its one-byte tag, and a variable-width payload.
fn split() -> FieldType {
    FieldType::Sum(vec![
        ctor(3, vec![scalar(Scalar::Text)]),
        ctor(4, vec![scalar(Scalar::I32)]),
    ])
}

/// A sum whose first constructor holds text and a sum, which is stored packed after the text.
fn nested() -> FieldType {
    let inner = FieldType::Sum(vec![
        ctor(5, vec![scalar(Scalar::I32)]),
        ctor(6, vec![scalar(Scalar::I32)]),
    ]);
    FieldType::Sum(vec![
        ctor(3, vec![scalar(Scalar::Text), inner]),
        ctor(4, Vec::new()),
    ])
}

fn nested_value(text: &str) -> Value {
    sum(
        0,
        vec![Value::Text(text.into()), sum(1, vec![Value::I32(7)])],
    )
}

/// Returns one case for each stored kind. A row of one variable-width field holds a two-byte end
/// offset and a one-byte bitmap before it, after a split sum's tag padded to two bytes.
fn cases() -> Vec<Case> {
    vec![
        Case {
            kind: "tag of a sum in a fixed row",
            row: vec![FieldType::Sum(vec![
                ctor(3, vec![scalar(Scalar::I32)]),
                ctor(4, vec![scalar(Scalar::I32)]),
            ])],
            value: vec![sum(1, vec![Value::I32(0x0A0B_0C0D)])],
            marker: 0x0A0B_0C0Di32.to_le_bytes().to_vec(),
            edit: |page, at| page[at - 4] = 0x77,
            store: false,
            cause: Undecoded::Tag,
        },
        Case {
            kind: "tag of a split sum",
            row: vec![split()],
            value: vec![sum(0, vec![Value::Text("splitmark".into())])],
            marker: b"splitmark".to_vec(),
            edit: |page, at| page[at - 6] = 0x77,
            store: false,
            cause: Undecoded::Tag,
        },
        Case {
            kind: "tag of a packed sum",
            row: vec![nested()],
            value: vec![nested_value("packmark")],
            marker: b"packmark".to_vec(),
            edit: |page, at| page[at + 8] = 0x70,
            store: false,
            cause: Undecoded::Tag,
        },
        Case {
            kind: "tag of a packed sum wider than 32 bits",
            row: vec![nested()],
            value: vec![nested_value("widemark")],
            marker: b"widemark".to_vec(),
            edit: |page, at| page[at + 8..at + 13].copy_from_slice(&[0xff, 0xff, 0xff, 0xff, 0x7f]),
            store: false,
            cause: Undecoded::Tag,
        },
        Case {
            kind: "text of a variable-width field",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("textmark".into())],
            marker: b"textmark".to_vec(),
            edit: |page, at| page[at] = 0xff,
            store: false,
            cause: Undecoded::Text,
        },
        Case {
            kind: "end offset of a variable-width field",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("cutmark".into())],
            marker: b"cutmark".to_vec(),
            edit: |page, at| page[at - 3..at - 1].copy_from_slice(&[0xff, 0xff]),
            store: false,
            cause: Undecoded::Cut,
        },
        Case {
            kind: "length of packed text",
            row: vec![split()],
            value: vec![sum(0, vec![Value::Text("trailmark".into())])],
            marker: b"trailmark".to_vec(),
            edit: |page, at| page[at - 1] = 7,
            store: false,
            cause: Undecoded::Trailing,
        },
        Case {
            kind: "chunk of an out-of-line pointer",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("x".repeat(9001))],
            marker: 9001u64.to_le_bytes().to_vec(),
            edit: |page, at| page[at + 8..at + 16].fill(0),
            store: false,
            cause: Undecoded::Pointer,
        },
        Case {
            kind: "out-of-line bit of an inline field",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("bitmark".into())],
            marker: b"bitmark".to_vec(),
            edit: |page, at| page[at - 1] = 1,
            store: false,
            cause: Undecoded::Pointer,
        },
        Case {
            kind: "length of an out-of-line pointer",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("x".repeat(9001))],
            marker: 9001u64.to_le_bytes().to_vec(),
            edit: |page, at| page[at..at + 8].fill(0xff),
            store: false,
            cause: Undecoded::Pointer,
        },
        Case {
            kind: "body shorter than its fixed part and offsets",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("shortmark".into())],
            marker: b"shortmark".to_vec(),
            edit: |page, at| {
                let start = u16::try_from(at - 3 - HEADER_LEN).unwrap();
                let len = u16::try_from(HEADER_LEN + 3 + 9).unwrap();
                let pointer = [start.to_le_bytes(), len.to_le_bytes()].concat();
                let slot = found(page, &pointer, "the line pointer");
                let short = u16::try_from(HEADER_LEN + 2).unwrap();
                page[slot + 2..slot + 4].copy_from_slice(&short.to_le_bytes());
            },
            store: false,
            cause: Undecoded::Cut,
        },
        Case {
            kind: "varint running past its field",
            row: vec![FieldType::Record(vec![scalar(Scalar::Text)])],
            value: vec![Value::Record(vec![Value::Text("vvvv".into())])],
            marker: b"vvvv".to_vec(),
            edit: |page, at| page[at - 1..at + 4].fill(0x80),
            store: false,
            cause: Undecoded::Cut,
        },
        Case {
            kind: "zone of a zoned date-time",
            row: vec![scalar(Scalar::ZonedDateTime)],
            value: vec![Value::ZonedDateTime {
                local: 0,
                zone: "zonemark".into(),
            }],
            marker: b"zonemark".to_vec(),
            edit: |page, at| page[at] = 0xff,
            store: false,
            cause: Undecoded::Text,
        },
        Case {
            kind: "text of an out-of-line value",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text(format!("chunkmark{}", "y".repeat(9000)))],
            marker: b"chunkmark".to_vec(),
            edit: |page, at| page[at] = 0xff,
            store: true,
            cause: Undecoded::Text,
        },
    ]
}

/// Returns where `marker` starts in `bytes`, which hold it once.
fn found(bytes: &[u8], marker: &[u8], kind: &str) -> usize {
    let starts: Vec<usize> = (0..=bytes.len() - marker.len())
        .filter(|&i| bytes[i..i + marker.len()] == *marker)
        .collect();
    assert_eq!(1, starts.len(), "{kind}: the marker is there once");
    starts[0]
}

/// The caller of a decode a case runs after verify.
#[derive(Clone, Copy, Debug)]
enum Via {
    Read,
    /// Every version of the heap, as `Heap::check_rows`, `scan_rooted` and `out_of_line` read
    /// them page by page.
    Scan,
    /// The chain from the version, as `chain_rows` reads a chain on one page.
    Chain,
    /// An update of the version, which decodes it unlatched as a retag and a conversion do.
    Update,
}

/// What breaking a case's byte did.
struct Broken {
    report: Report,
    /// The outcome of reading the row through the case's caller.
    read: Result<(), PageError>,
    /// The outcome of reading the row inserted beside it.
    beside: Result<(), PageError>,
    /// The page of the row, and the page the case's marker is on.
    row: Pid,
    marked: Pid,
}

/// Returns what breaking the byte `case` names does to verify and to reading its row through
/// `via`.
fn broken(case: &Case, via: Via) -> Broken {
    let mut out = None;
    let mut sim = Sim::new(1, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(FRAMES)).unwrap();
        let txns = Transactions::create(rt, &pager).unwrap();
        let large = LargeStore::create(&pager, 9).unwrap();
        let heap = Heap::create(&pager, 10, case.row.clone(), large).unwrap();
        let begun = txns.begin(&pager).unwrap();
        let beside = heap.insert(&pager, begun.xid(), &case.value).unwrap();
        let tid = heap.insert(&pager, begun.xid(), &case.value).unwrap();
        txns.commit(&pager, begun).unwrap();
        let row = Pid(tid.page());
        assert_eq!(
            row,
            Pid(beside.page()),
            "{}: both rows share a page",
            case.kind
        );
        let at = if case.store {
            heap.pointers_at(&pager, tid).unwrap().unwrap()[0].first
        } else {
            tid
        };
        let marked = Pid(at.page());
        let slot = {
            let guard = pager.read(marked).unwrap();
            HeapPage::any(&guard).unwrap().get(usize::from(at.slot()))
        };
        let Slot::Normal { at: bytes, .. } = slot else {
            panic!("{}: the slot holds a version", case.kind);
        };
        pager
            .update(&[marked], |p| {
                let at = bytes.start + found(&p[0][bytes], &case.marker, case.kind);
                (case.edit)(p[0], at);
            })
            .unwrap();
        pager.flush_log();
        let clog = txns.commit_log();
        let inventory = Inventory {
            commit_log: Some(&clog),
            sums: vec![Sum {
                heaps: vec![(CtorId(0), &heap)],
                indexes: Vec::new(),
            }],
            trees: Vec::new(),
            pages: Vec::new(),
        };
        let report = verify::verify(&pager, &inventory);
        let read = match via {
            Via::Read => heap.read(&pager, tid).map(|_| ()),
            Via::Scan => heap.scan(&pager, drop),
            Via::Chain => heap.chain_rows(&pager, tid).map(|_| ()),
            Via::Update => {
                let begun = txns.begin(&pager).unwrap();
                let updated = heap.update(&pager, tid, begun.xid(), &case.value, &[]);
                updated.map(|_| ()).map_err(|e| match e {
                    Error::Page(e) => e,
                    e => panic!("{}: an update fails on the page, not {e:?}", case.kind),
                })
            }
        };
        let beside = heap.read(&pager, beside).map(|_| ());
        out = Some(Broken {
            report,
            read,
            beside,
            row,
            marked,
        });
    });
    out.expect("the run finished")
}

/// Returns the cause after `c` in a fixed order, `None` after the last.
fn next_cause(c: Undecoded) -> Option<Undecoded> {
    match c {
        Undecoded::Tag => Some(Undecoded::Cut),
        Undecoded::Cut => Some(Undecoded::Trailing),
        Undecoded::Trailing => Some(Undecoded::Text),
        Undecoded::Text => Some(Undecoded::Pointer),
        Undecoded::Pointer => None,
    }
}

/// Assert that breaking `case` through `via` is reported whole, and return the cause it reached.
fn reached(case: &Case, via: Via) -> Undecoded {
    let what = format!("{} through {via:?}", case.kind);
    let b = broken(case, via);
    let damage = Damage::Shape(Shape::Row(case.cause));
    let finding = Finding {
        pid: b.marked,
        problem: Problem::Damaged(damage),
    };
    assert_eq!(vec![finding], b.report.findings, "{what}: verify");
    assert_eq!(
        Err(PageError {
            pid: b.marked,
            damage
        }),
        b.read,
        "{what}: read"
    );
    if case.store {
        assert_ne!(b.row, b.marked, "{what}: the value lies on a store page");
        assert_eq!(Ok(()), b.beside, "{what}: the row beside it");
    }
    match b.read {
        Err(PageError {
            damage: Damage::Shape(Shape::Row(cause)),
            ..
        }) => cause,
        other => panic!("{what}: {other:?}"),
    }
}

#[test]
fn a_row_that_does_not_decode_is_reported_as_damage_of_its_page() {
    let mut causes = BTreeSet::new();
    for case in &cases() {
        causes.insert(reached(case, Via::Read));
    }
    let every: BTreeSet<Undecoded> =
        std::iter::successors(Some(Undecoded::Tag), |&c| next_cause(c)).collect();
    assert_eq!(every, causes, "every cause is reached");
}

#[test]
fn each_caller_family_refuses_the_page_of_a_row_that_does_not_decode() {
    let cases = cases();
    let case = cases
        .iter()
        .find(|c| c.kind == "tag of a split sum")
        .expect("the case is listed");
    for via in [Via::Scan, Via::Chain, Via::Update] {
        assert_eq!(case.cause, reached(case, via), "{via:?}");
    }
}
