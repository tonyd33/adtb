use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::rc::Rc;

use crate::heap::handle::before_next_row_refusal;
use crate::heap::large::before_next_tail;
use crate::heap::page::{HeapPage, Slot};
use crate::heap::version::tid_bits;
use crate::heap::{
    CtorType, Error, FieldType, HEADER_LEN, Heap, HeapNo, LargeStore, Refused, walk,
};
use crate::key::{CtorId, Scalar, Tid, Value};
use crate::mvcc::{Fate, Transactions, Xid};
use crate::storage::{Damage, PageError, Pager, Pid, Shape, Undecoded, Walk};
use crate::verify::{self, Finding, Inventory, Problem, Report, Sum};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::process::finished;

use crate::test_world::{PAGE, options};

const FRAMES: usize = 32;

/// One stored row kind, the row that holds it, and how to break the byte that decides it.
struct Case {
    kind: &'static str,
    row: Vec<FieldType>,
    value: Vec<Value>,
    /// The bytes found once in the version or chunk `marked` names, and the edit to its page.
    marker: Vec<u8>,
    edit: fn(&mut [u8], &Mark),
    marked: On,
    /// The page the damage is found on.
    blamed: On,
    expect: Expect,
}

/// A version of the row, or a chunk of its out-of-line value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum On {
    Row,
    Chunk(usize),
    Last,
}

/// Where a case's marker lies on its page.
struct Mark {
    at: usize,
    /// The bytes of the version or chunk holding it.
    tuple: Range<usize>,
    tid: Tid,
}

/// What breaking a case's byte is found as.
#[derive(Clone, Copy)]
enum Expect {
    /// A row that does not decode.
    Row(Undecoded),
    /// A slot table out of frame: verify finds what the function makes of the marked tuple's
    /// bytes as they were, and a read refuses the page.
    Frame(fn(Range<usize>) -> String),
}

/// Set the length of the marked version's line pointer to `len`.
fn set_len(page: &mut [u8], m: &Mark, len: usize) {
    let p = line_pointer(page, m);
    page[p + 2..p + 4].copy_from_slice(&u16::try_from(len).unwrap().to_le_bytes());
}

/// Returns where the line pointer of the marked version starts.
fn line_pointer(page: &[u8], m: &Mark) -> usize {
    let start = u16::try_from(m.tuple.start).unwrap();
    let len = u16::try_from(m.tuple.len()).unwrap();
    let pointer = [start.to_le_bytes(), len.to_le_bytes()].concat();
    found(page, &pointer, "the line pointer")
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

/// Returns one case for each stored kind of the row and its pages.
fn cases() -> Vec<Case> {
    let mut cases = row_cases();
    cases.extend(frame_cases());
    cases.extend(chunk_cases());
    cases.extend(domain_cases());
    cases
}

/// Returns one case of each fixed-width scalar with a bounded domain, its value moved outside it.
fn domain_cases() -> Vec<Case> {
    vec![
        Case {
            kind: "date past the last day",
            row: vec![scalar(Scalar::Date)],
            value: vec![Value::Date(0x0A0B_0C0D)],
            marker: 0x0A0B_0C0Di32.to_le_bytes().to_vec(),
            edit: |page, m| page[m.at..m.at + 4].copy_from_slice(&i32::MAX.to_le_bytes()),
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Domain),
        },
        Case {
            kind: "time before midnight",
            row: vec![scalar(Scalar::Time)],
            value: vec![Value::Time(0x11_2233_4455)],
            marker: 0x11_2233_4455i64.to_le_bytes().to_vec(),
            edit: |page, m| page[m.at..m.at + 8].copy_from_slice(&(-1i64).to_le_bytes()),
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Domain),
        },
        Case {
            kind: "decimal of 39 digits",
            row: vec![scalar(Scalar::Decimal(0))],
            value: vec![Value::Decimal(0x0A0B_0C0D_0E0F_1011)],
            marker: 0x0A0B_0C0D_0E0F_1011i128.to_le_bytes().to_vec(),
            edit: |page, m| page[m.at..m.at + 16].copy_from_slice(&10i128.pow(38).to_le_bytes()),
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Domain),
        },
    ]
}

/// Returns one case for each stored kind. A row of one variable-width field holds a two-byte end
/// offset and a one-byte bitmap before it, after a split sum's tag padded to two bytes.
fn row_cases() -> Vec<Case> {
    vec![
        Case {
            kind: "tag of a sum in a fixed row",
            row: vec![FieldType::Sum(vec![
                ctor(3, vec![scalar(Scalar::I32)]),
                ctor(4, vec![scalar(Scalar::I32)]),
            ])],
            value: vec![sum(1, vec![Value::I32(0x0A0B_0C0D)])],
            marker: 0x0A0B_0C0Di32.to_le_bytes().to_vec(),
            edit: |page, m| page[m.at - 4] = 0x77,
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Tag),
        },
        Case {
            kind: "tag of a split sum",
            row: vec![split()],
            value: vec![sum(0, vec![Value::Text("splitmark".into())])],
            marker: b"splitmark".to_vec(),
            edit: |page, m| page[m.at - 6] = 0x77,
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Tag),
        },
        Case {
            kind: "tag of a packed sum",
            row: vec![nested()],
            value: vec![nested_value("packmark")],
            marker: b"packmark".to_vec(),
            edit: |page, m| page[m.at + 8] = 0x70,
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Tag),
        },
        Case {
            kind: "tag of a packed sum wider than 32 bits",
            row: vec![nested()],
            value: vec![nested_value("widemark")],
            marker: b"widemark".to_vec(),
            edit: |page, m| {
                page[m.at + 8..m.at + 13].copy_from_slice(&[0xff, 0xff, 0xff, 0xff, 0x7f]);
            },
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Tag),
        },
        Case {
            kind: "text of a variable-width field",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("textmark".into())],
            marker: b"textmark".to_vec(),
            edit: |page, m| page[m.at] = 0xff,
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Text),
        },
        Case {
            kind: "end offset of a variable-width field",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("cutmark".into())],
            marker: b"cutmark".to_vec(),
            edit: |page, m| page[m.at - 3..m.at - 1].copy_from_slice(&[0xff, 0xff]),
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Cut),
        },
        Case {
            kind: "length of packed text",
            row: vec![split()],
            value: vec![sum(0, vec![Value::Text("trailmark".into())])],
            marker: b"trailmark".to_vec(),
            edit: |page, m| page[m.at - 1] = 7,
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Trailing),
        },
        Case {
            kind: "chunk of an out-of-line pointer",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("x".repeat(9001))],
            marker: 9001u64.to_le_bytes().to_vec(),
            edit: |page, m| page[m.at + 8..m.at + 16].fill(0),
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Pointer),
        },
        Case {
            kind: "out-of-line bit of an inline field",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("bitmark".into())],
            marker: b"bitmark".to_vec(),
            edit: |page, m| page[m.at - 1] = 1,
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Pointer),
        },
        Case {
            kind: "length of an out-of-line pointer",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("x".repeat(9001))],
            marker: 9001u64.to_le_bytes().to_vec(),
            edit: |page, m| page[m.at..m.at + 8].fill(0xff),
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Pointer),
        },
        Case {
            kind: "body shorter than its fixed part and offsets",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("shortmark".into())],
            marker: b"shortmark".to_vec(),
            edit: |page, m| set_len(page, m, HEADER_LEN + 2),
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Cut),
        },
        Case {
            kind: "varint running past its field",
            row: vec![FieldType::Record(vec![scalar(Scalar::Text)])],
            value: vec![Value::Record(vec![Value::Text("vvvv".into())])],
            marker: b"vvvv".to_vec(),
            edit: |page, m| page[m.at - 1..m.at + 4].fill(0x80),
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Cut),
        },
        Case {
            kind: "zone of a zoned date-time",
            row: vec![scalar(Scalar::ZonedDateTime)],
            value: vec![Value::ZonedDateTime {
                local: 0,
                zone: "zonemark".into(),
            }],
            marker: b"zonemark".to_vec(),
            edit: |page, m| page[m.at] = 0xff,
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Text),
        },
        Case {
            kind: "text of an out-of-line value",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text(format!("chunkmark{}", "y".repeat(9000)))],
            marker: b"chunkmark".to_vec(),
            edit: |page, m| page[m.at] = 0xff,
            marked: On::Chunk(0),
            blamed: On::Chunk(0),
            expect: Expect::Row(Undecoded::Text),
        },
    ]
}

/// Returns one case for each way a line pointer or the tuples of a slotted page leave its frame.
fn frame_cases() -> Vec<Case> {
    vec![
        Case {
            kind: "state of a line pointer",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("statemark".into())],
            marker: b"statemark".to_vec(),
            edit: |page, m| page[line_pointer(page, m) + 4] = 9,
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Frame(|_| "slot 1 has state 9".into()),
        },
        Case {
            kind: "slot shorter than a version header",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("headmark".into())],
            marker: b"headmark".to_vec(),
            edit: |page, m| set_len(page, m, HEADER_LEN - 1),
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Frame(|t| {
                format!(
                    "slot 1 holds bytes {}..{}",
                    t.start,
                    t.start + HEADER_LEN - 1
                )
            }),
        },
        Case {
            kind: "slot running past the page",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("pastmark".into())],
            marker: b"pastmark".to_vec(),
            edit: |page, m| {
                let p = line_pointer(page, m);
                page[p..p + 2].copy_from_slice(&u16::try_from(PAGE - 8).unwrap().to_le_bytes());
            },
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Frame(|t| {
                format!("slot 1 holds bytes {}..{}", PAGE - 8, PAGE - 8 + t.len())
            }),
        },
        Case {
            kind: "tuples longer than the page",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("longmark".into())],
            marker: b"longmark".to_vec(),
            edit: |page, m| {
                let p = line_pointer(page, m);
                let lower = p + 8;
                page[28..32].copy_from_slice(&u32::try_from(lower).unwrap().to_le_bytes());
                page[p..p + 2].copy_from_slice(&u16::try_from(lower).unwrap().to_le_bytes());
                let len = u16::try_from(PAGE - lower).unwrap();
                page[p + 2..p + 4].copy_from_slice(&len.to_le_bytes());
            },
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Frame(|_| "slots 1 and 0 overlap".into()),
        },
    ]
}

/// Returns one case for each way the chunks of an out-of-line value fail it.
fn chunk_cases() -> Vec<Case> {
    vec![
        Case {
            kind: "length of an out-of-line value past its chunks",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("x".repeat(9001))],
            marker: 9001u64.to_le_bytes().to_vec(),
            edit: |page, m| page[m.at..m.at + 8].copy_from_slice(&9002u64.to_le_bytes()),
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Pointer),
        },
        Case {
            kind: "length of an out-of-line value short of its chunks",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text("x".repeat(9001))],
            marker: 9001u64.to_le_bytes().to_vec(),
            edit: |page, m| page[m.at..m.at + 8].copy_from_slice(&9000u64.to_le_bytes()),
            marked: On::Row,
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Pointer),
        },
        Case {
            kind: "next of a chunk naming itself",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text(format!("{}selfmark", "x".repeat(9000)))],
            marker: b"selfmark".to_vec(),
            edit: |page, m| {
                let next = m.tuple.start;
                page[next..next + 8].copy_from_slice(&tid_bits(m.tid).to_le_bytes());
            },
            marked: On::Last,
            blamed: On::Last,
            expect: Expect::Row(Undecoded::Trailing),
        },
        Case {
            kind: "next of a chunk naming an unused slot",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text(format!("slotmark{}", "x".repeat(9000)))],
            marker: b"slotmark".to_vec(),
            edit: |page, m| {
                let unused = Tid::new(m.tid.page(), 200).unwrap();
                let next = m.tuple.start;
                page[next..next + 8].copy_from_slice(&tid_bits(unused).to_le_bytes());
            },
            marked: On::Chunk(0),
            blamed: On::Chunk(0),
            expect: Expect::Row(Undecoded::Pointer),
        },
        Case {
            kind: "role of a later chunk",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text(format!(
                "{}rolemark{}",
                "y".repeat(5000),
                "y".repeat(4000)
            ))],
            marker: b"rolemark".to_vec(),
            edit: |page, m| page[m.tuple.start + 20..m.tuple.start + 24].fill(0),
            marked: On::Chunk(1),
            blamed: On::Chunk(0),
            expect: Expect::Row(Undecoded::Pointer),
        },
        Case {
            kind: "role of a first chunk",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text(format!("headmark{}", "x".repeat(9000)))],
            marker: b"headmark".to_vec(),
            edit: |page, m| page[m.tuple.start + 20] = 1,
            marked: On::Chunk(0),
            blamed: On::Row,
            expect: Expect::Row(Undecoded::Pointer),
        },
        Case {
            kind: "text of a middle chunk",
            row: vec![scalar(Scalar::Text)],
            value: vec![Value::Text(format!(
                "{}midmark{}",
                "y".repeat(5000),
                "y".repeat(4000)
            ))],
            marker: b"midmark".to_vec(),
            edit: |page, m| page[m.at] = 0xff,
            marked: On::Chunk(1),
            blamed: On::Chunk(1),
            expect: Expect::Row(Undecoded::Text),
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

/// The caller of a decode a case runs.
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
    /// A prune of the row's page removing every version on it.
    Prune,
}

/// What breaking a case's byte did.
struct Broken {
    /// Verify's report, when it ran in place of a caller.
    report: Option<Report>,
    /// The outcome of reading the row through the case's caller, when one ran.
    read: Result<(), PageError>,
    /// The outcome of reading the row inserted beside it, when no page of it is the blamed page.
    beside: Option<Result<(), PageError>>,
    /// The page the case blames, and the bytes of the marked tuple before the edit.
    blamed: Pid,
    tuple: Range<usize>,
}

/// Returns each page holding the row at `tid` or a chunk of its out-of-line value, the row's
/// first.
fn pages_of(pager: &Pager<SimRuntime>, heap: &Heap, tid: Tid) -> Vec<Tid> {
    let mut out = vec![tid];
    for p in heap.pointers_at(pager, tid).unwrap().unwrap() {
        out.extend(
            heap.large()
                .chunks(pager, p)
                .unwrap()
                .into_iter()
                .map(|(t, _)| t),
        );
    }
    out
}

/// Returns what breaking the byte `case` names does to reading its row through `via`, or to
/// verify when `via` is `None`.
fn broken(case: &Case, via: Option<Via>) -> Broken {
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
        assert_eq!(
            Pid(tid.page()),
            Pid(beside.page()),
            "{}: both rows share a page",
            case.kind
        );
        let tids = pages_of(&pager, &heap, tid);
        let place = |on| match on {
            On::Row => tids[0],
            On::Chunk(k) => tids[k + 1],
            On::Last => *tids.last().unwrap(),
        };
        let at = place(case.marked);
        let blamed = Pid(place(case.blamed).page());
        let marked = Pid(at.page());
        let apart = pages_of(&pager, &heap, beside)
            .iter()
            .all(|t| Pid(t.page()) != blamed);
        let slot = {
            let guard = pager.read(marked).unwrap();
            HeapPage::any(&guard).unwrap().get(usize::from(at.slot()))
        };
        let Slot::Normal { at: tuple, .. } = slot else {
            panic!("{}: the slot holds a version", case.kind);
        };
        pager
            .update(&[marked], |p| {
                let mark = Mark {
                    at: tuple.start + found(&p[0][tuple.clone()], &case.marker, case.kind),
                    tuple: tuple.clone(),
                    tid: at,
                };
                (case.edit)(p[0], &mark);
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
        let Some(via) = via else {
            out = Some(Broken {
                report: Some(verify::verify(&pager, &inventory)),
                read: Ok(()),
                beside: None,
                blamed,
                tuple,
            });
            return;
        };
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
            Via::Prune => {
                let removed = heap.prune(&pager, marked, |_| Ok(Fate::Remove), &[], false);
                removed.map(|_| ()).map_err(|e| match e {
                    Refused::Page(e) => e,
                    Refused::Gone => panic!("{}: a prune refuses the page", case.kind),
                })
            }
        };
        let beside = apart.then(|| heap.read(&pager, beside).map(|_| ()));
        out = Some(Broken {
            report: None,
            read,
            beside,
            blamed,
            tuple,
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
        Undecoded::Pointer => Some(Undecoded::Domain),
        Undecoded::Domain => None,
    }
}

/// Assert that breaking `case`, in one run, is reported whole by verify, and in another, refused
/// through `via`. Returns the shape the read reached.
fn reached(case: &Case, via: Via) -> Shape {
    let what = format!("{} through {via:?}", case.kind);
    let v = broken(case, None);
    let (shape, problem) = match case.expect {
        Expect::Row(cause) => (
            Shape::Row(cause),
            Problem::Damaged(Damage::Shape(Shape::Row(cause))),
        ),
        Expect::Frame(found) => (
            Shape::Frame,
            Problem::Structure(format!("heap 10: {}", found(v.tuple.clone()))),
        ),
    };
    let finding = Finding {
        pid: v.blamed,
        problem,
    };
    let report = v.report.expect("verify ran");
    assert_eq!(vec![finding], report.findings, "{}: verify", case.kind);
    let damage = Damage::Shape(shape);
    let b = broken(case, Some(via));
    assert_eq!(
        Err(PageError {
            pid: b.blamed,
            damage
        }),
        b.read,
        "{what}: read"
    );
    if let Some(beside) = b.beside {
        assert_eq!(Ok(()), beside, "{what}: the row beside it");
    }
    match b.read {
        Err(PageError {
            damage: Damage::Shape(shape),
            ..
        }) => shape,
        other => panic!("{what}: {other:?}"),
    }
}

fn case(kind: &str) -> Case {
    cases()
        .into_iter()
        .find(|c| c.kind == kind)
        .expect("the case is listed")
}

#[test]
fn a_page_that_does_not_decode_is_reported_as_damage_of_its_page() {
    let mut shapes = BTreeSet::new();
    for case in &cases() {
        shapes.insert(reached(case, Via::Read));
    }
    let every: BTreeSet<Shape> = std::iter::successors(Some(Undecoded::Tag), |&c| next_cause(c))
        .map(Shape::Row)
        .chain([Shape::Frame])
        .collect();
    assert_eq!(every, shapes, "every shape is reached");
}

#[test]
fn each_caller_family_refuses_the_page_of_a_row_that_does_not_decode() {
    let case = case("tag of a split sum");
    for via in [Via::Scan, Via::Chain, Via::Update] {
        assert_eq!(Shape::Row(Undecoded::Tag), reached(&case, via), "{via:?}");
    }
    let unframed = self::case("state of a line pointer");
    assert_eq!(Shape::Frame, reached(&unframed, Via::Scan));
}

#[test]
fn a_prune_refuses_the_page_of_a_version_it_frees_that_does_not_decode() {
    let case = case("end offset of a variable-width field");
    assert_eq!(Shape::Row(Undecoded::Cut), reached(&case, Via::Prune));
}

#[test]
fn a_chain_walk_whose_refusal_finds_the_page_mended_reads_the_chain_again_whole() {
    let mut sim = Sim::new(1, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options(FRAMES)).unwrap();
        let large = LargeStore::create(&pager, 9).unwrap();
        let heap = Heap::create(&pager, 10, vec![split()], large).unwrap();
        let first = heap
            .insert(
                &pager,
                Xid(1),
                &[sum(0, vec![Value::Text("firstmark".into())])],
            )
            .unwrap();
        let second = heap
            .update(
                &pager,
                first,
                Xid(2),
                &[sum(0, vec![Value::Text("hotmark".into())])],
                &[],
            )
            .unwrap();
        assert!(second.heap_only, "the update stays on the page");
        let pid = Pid(first.page());
        let tag = {
            let guard = pager.read(pid).unwrap();
            let page = HeapPage::any(&guard).unwrap();
            let Slot::Normal { at, .. } = page.get(usize::from(second.tid.slot())) else {
                panic!("the slot holds a version");
            };
            at.start + found(&page[at], b"hotmark", "the second version") - 6
        };
        let was = pager.read(pid).unwrap()[tag];
        pager.update(&[pid], |p| p[0][tag] = 0x77).unwrap();
        let mender = pager.clone();
        before_next_row_refusal(move || {
            mender.update(&[pid], |p| p[0][tag] = was).unwrap();
        });
        let mut walked = Vec::new();
        let none = walk(&pager, heap.id(), &[&heap], first, |link, _| {
            walked.push(link.tid);
            Ok::<_, PageError>(false)
        });
        assert_eq!(Ok(None), none.map(|found| found.map(|(link, _)| link.tid)));
        assert_eq!(vec![first, second.tid], walked);
        assert_eq!(BTreeMap::new(), pager.damaged());
    });
}

#[test]
fn a_sweep_refuses_a_touched_page_out_of_frame() {
    type Break = (&'static str, fn(&mut [u8]), Shape);
    let breaks: [Break; 2] = [
        ("a line pointer's state", |page| page[44] = 9, Shape::Frame),
        ("the page kind", |page| page[14..16].fill(0xee), Shape::Kind),
    ];
    for (what, edit, shape) in breaks {
        let mut sim = Sim::new(1, Config::default());
        finished(&mut sim, |rt| {
            let pager = Pager::create(rt.clone(), &options(FRAMES)).unwrap();
            let txns = Transactions::create(rt, &pager).unwrap();
            let large = LargeStore::create(&pager, 9).unwrap();
            let pointer = large
                .write(&pager, (Xid(1), HeapNo(10)), b"sweepmark")
                .unwrap();
            let pid = Pid(pointer.first.page());
            pager.update(&[pid], |p| edit(p[0])).unwrap();
            let blamed = if shape == Shape::Kind {
                large.root()
            } else {
                pid
            };
            let damage = Damage::Shape(shape);
            let got = large.sweep(
                &crate::vacuum::Vacuuming::unheld(&pager, &txns),
                |_| Ok(true),
                &mut || true,
            );
            assert_eq!(
                Err::<Walk, _>(PageError {
                    pid: blamed,
                    damage
                }),
                got,
                "{what}"
            );
            assert_eq!(
                BTreeMap::from([(blamed, damage)]),
                pager.damaged(),
                "{what}"
            );
        });
    }
}

#[test]
fn a_chunk_insert_refuses_the_page_of_a_tail_it_cannot_find() {
    let mut sim = Sim::new(1, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options(FRAMES)).unwrap();
        let large = LargeStore::create(&pager, 9).unwrap();
        let tail = Rc::new(Cell::new(None));
        let seen = Rc::clone(&tail);
        before_next_tail(move |t, page| {
            page[40 + 8 * usize::from(t.slot()) + 4] = 9;
            seen.set(Some(t));
        });
        let got = large.write(&pager, (Xid(1), HeapNo(10)), &[7; 5000]);
        let pid = Pid(tail.get().expect("the hook ran").page());
        let damage = Damage::Shape(Shape::Frame);
        assert_eq!(Err(Error::Page(PageError { pid, damage })), got.map(|_| ()));
        assert_eq!(BTreeMap::from([(pid, damage)]), pager.damaged());
    });
}
