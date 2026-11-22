//! In-shape damage of an index: a tree page whose checksum holds over contents no write of the
//! engine gives it, which every reader reaching it must refuse naming the page and mark its
//! index, never panic or loop on.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

use adtb::CallOptions;
use adtb::key::Value;
use adtb::testing::{Damage, Pid, Quarantine, Shape, page};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::process;
use adtb_testkit::search::Files;

use super::world::{self, DATA, Op, Rel};

const SEED: u64 = 29;

/// The sequence each case reshapes the file of: enough rows of `users` that its key index has a
/// root over several leaves, the even keys only, closed cleanly.
fn ops() -> Vec<Op> {
    vec![
        Op::Insert(Rel::Users, (1..=600).map(|i| 2 * i).collect()),
        Op::Checkpoint,
    ]
}

const RIGHT: usize = page::HEADER_LEN;
const COUNT: usize = page::HEADER_LEN + 8;
const HIGH: usize = page::HEADER_LEN + 14;
const SLOTS: usize = page::HEADER_LEN + 18;
const OWNER: usize = 16;
const FLAGS: usize = 20;
const DELETED: u16 = 1 << 2;

/// How a case reshapes the leftmost leaf of the key index of `users`, keeping its checksum sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reshape {
    /// The first slot naming an offset past the page's end.
    SlotPastPage,
    /// The cell of the second entry, key 4, a byte shorter: an entry that does not decode.
    EntryCut,
    /// Slots added past the count, each naming the first cell, so the cells outgrow the page.
    Overlapping,
    /// The right link naming the leaf itself, and the high key lowered to the first entry, so a
    /// key above it moves right to the leaf itself.
    LoopedLink,
    /// The owner naming another structure: the root's downlink names a page of another shape.
    Owner,
    /// The leaf flagged unlinked while its parent still names it.
    Deleted,
}

const RESHAPES: [Reshape; 6] = [
    Reshape::SlotPastPage,
    Reshape::EntryCut,
    Reshape::Overlapping,
    Reshape::LoopedLink,
    Reshape::Owner,
    Reshape::Deleted,
];

/// Statements reading the reshaped leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Reader {
    /// A query probing the key index for key 4.
    Probe,
    /// An insert of key 3, probing the key index for it and adding its entry to the leaf.
    Insert,
    /// A vacuum after a delete of key 4, removing its entry from the leaf.
    Vacuum,
    /// A vacuum after a delete of every key of the leaf, unlinking the emptied leaf.
    Unlink,
}

const READERS: [Reader; 4] = [
    Reader::Probe,
    Reader::Insert,
    Reader::Vacuum,
    Reader::Unlink,
];

impl Reshape {
    /// Returns the readers that reach the fault, and the page they refuse with its shape: the
    /// leaf for a fault of its own bytes, the root naming it for any other. Every other reader
    /// sees the rows it asks for.
    fn refused(self, leaf: Pid, root: Pid) -> (&'static [Reader], Pid, Shape) {
        use Reader::*;
        match self {
            Reshape::SlotPastPage | Reshape::LoopedLink | Reshape::Overlapping => {
                (&[Probe, Insert, Vacuum, Unlink], leaf, Shape::Frame)
            }
            Reshape::EntryCut => (&[Probe, Vacuum, Unlink], leaf, Shape::Frame),
            Reshape::Owner => (&[Probe, Insert, Vacuum, Unlink], root, Shape::Owner),
            Reshape::Deleted => (&[Probe, Insert, Vacuum, Unlink], root, Shape::Frame),
        }
    }
}

impl Reshape {
    /// Returns whether a probe of key 2, the leaf's first, misses the fault: it lies in the cell
    /// of key 4 or in the high key, which is key 2's entry and so above key 2.
    fn spares_first(self) -> bool {
        matches!(self, Reshape::EntryCut | Reshape::LoopedLink)
    }
}

impl Reader {
    /// Returns the statements of the reader, each run alone, over a leaf holding the even keys
    /// up to `last`.
    fn statements(self, last: i64) -> Vec<String> {
        let delete = |k: i64| format!("delete u in users where key u == users.key({k})");
        match self {
            Reader::Probe => {
                vec!["from u in users where key u == users.key(4) select u.name".to_owned()]
            }
            Reader::Insert => vec![r#"insert users { id: users.key(3), name: "n" }"#.to_owned()],
            Reader::Vacuum => vec![delete(4), "vacuum users".to_owned()],
            Reader::Unlink => (1..=last / 2)
                .map(|i| delete(2 * i))
                .chain(["vacuum users".to_owned()])
                .collect(),
        }
    }
}

fn u16_at(bytes: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
}

fn put_u16(bytes: &mut [u8], at: usize, v: usize) {
    let v = u16::try_from(v).expect("a page field fits in 16 bits");
    bytes[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

/// Returns `files` with `leaf` reshaped as `reshape` says and sealed.
fn reshaped(files: &Files, leaf: Pid, reshape: Reshape) -> Files {
    let mut files = files.clone();
    let bytes = DATA.page_mut(&mut files, leaf);
    let count = u16_at(bytes, COUNT);
    let first = u16_at(bytes, SLOTS);
    match reshape {
        Reshape::SlotPastPage => put_u16(bytes, SLOTS, world::PAGE - 1),
        Reshape::EntryCut => {
            let cell = u16_at(bytes, SLOTS + 2);
            let len = u16_at(bytes, cell);
            put_u16(bytes, cell, len - 1);
        }
        Reshape::Overlapping => {
            for i in count..count + 200 {
                put_u16(bytes, SLOTS + 2 * i, first);
            }
            put_u16(bytes, COUNT, count + 200);
        }
        Reshape::LoopedLink => {
            bytes[RIGHT..RIGHT + 8].copy_from_slice(&leaf.0.to_le_bytes());
            put_u16(bytes, HIGH, first);
        }
        Reshape::Owner => bytes[OWNER] ^= 1,
        Reshape::Deleted => {
            let flags = u16_at(bytes, FLAGS) | usize::from(DELETED);
            put_u16(bytes, FLAGS, flags);
        }
    }
    page::seal(leaf, bytes);
    files
}

/// The pages of the key index of `users` a case reshapes and blames, its mark, and the last key
/// of the leaf.
struct Layout {
    root: Pid,
    leaf: Pid,
    mark: Quarantine,
    last: i64,
}

/// Returns the layout of the key index of `users` in `files`, as an open of them reads it.
fn layout(files: Files) -> Layout {
    let mut sim = Sim::with_files(SEED, Config::default(), files);
    let mut out = None;
    process::run(&mut sim, |rt| {
        let (open, _) = world::open_own(&rt, 0, &world::options(), false);
        let db = open.expect("the clean file opens");
        let t = db.testing();
        let index = t.index("app::users", "key id").expect("the key index");
        let entry = t.entries(index, &[Value::I64(2)]).unwrap().remove(0);
        let leaf = t
            .leaf_of(index, &entry)
            .unwrap()
            .expect("a leaf holds key 2");
        let root = t.root(index).unwrap();
        assert_ne!(root, leaf, "the key index has a root over its leaves");
        let held = |k: i64| {
            let entry = t.entries(index, &[Value::I64(k)]).unwrap().remove(0);
            t.leaf_of(index, &entry).unwrap() == Some(leaf)
        };
        let last = (1..)
            .map(|i| 2 * i)
            .take_while(|&k| held(k))
            .last()
            .unwrap();
        assert!(
            !held(last + 2) && last < 1200,
            "the leaf holds a prefix of the keys"
        );
        out = Some(Layout {
            root,
            leaf,
            mark: t.own_mark(index).unwrap(),
            last,
        });
        db.close().expect("the clean file closes");
    });
    out.expect("the open ran")
}

/// What a reader of a reshaped file showed.
#[derive(Debug, PartialEq, Eq)]
struct Seen {
    /// The replies to the statements in order, each run of equal replies once.
    got: Vec<String>,
    /// The damage the pager recorded of each page.
    recorded: BTreeMap<Pid, Damage>,
    marks: BTreeSet<Quarantine>,
    panic: Option<String>,
}

/// Returns what an open of `files` and `statements` show.
fn read(files: Files, statements: &[String]) -> Seen {
    let mut sim = Sim::with_files(SEED, Config::default(), files);
    let mut seen = Seen {
        got: Vec::new(),
        recorded: BTreeMap::new(),
        marks: BTreeSet::new(),
        panic: None,
    };
    let ran = catch_unwind(AssertUnwindSafe(|| {
        process::run(&mut sim, |rt| {
            let (open, _) = world::open_own(&rt, 0, &world::options(), false);
            let db = open.expect("the reshaped file opens");
            for text in statements {
                let got = format!(
                    "{:?}",
                    db.query_text(text, &[], &CallOptions::default()).map(drop)
                );
                if seen.got.last() != Some(&got) {
                    seen.got.push(got);
                }
            }
            let probe = db.testing().probe();
            seen.recorded = probe.records().pages().clone();
            seen.marks = probe.marks();
            drop(db.close());
        });
    }));
    if let Err(payload) = ran {
        seen.panic = Some(
            (payload.downcast_ref::<String>().cloned())
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "a panic".to_owned()),
        );
    }
    seen
}

#[test]
fn an_index_page_out_of_shape_under_a_sound_checksum_is_refused_and_marked() {
    let arm = |_: &SimRuntime| {};
    let ran = world::run(SEED, &ops(), &arm, false, false, false);
    assert_eq!(BTreeMap::new(), ran.failed, "the sequence runs clean");
    let at = layout(ran.files.clone());
    let quarantined = r#"Err(Fatal { reason: Quarantined { what: "app::users key id" } })"#;
    let mut got = Vec::new();
    let mut want = Vec::new();
    for reshape in RESHAPES {
        let files = reshaped(&ran.files, at.leaf, reshape);
        let (readers, pid, shape) = reshape.refused(at.leaf, at.root);
        for reader in READERS {
            let statements = reader.statements(at.last);
            got.push((reshape, reader, read(files.clone(), &statements)));
            let seen = if readers.contains(&reader) {
                let mut replies = Vec::new();
                if reader == Reader::Unlink && reshape.spares_first() {
                    replies.push("Ok(())".to_owned());
                }
                replies.push(quarantined.to_owned());
                Seen {
                    got: replies,
                    recorded: BTreeMap::from([(pid, Damage::Shape(shape))]),
                    marks: BTreeSet::from([at.mark]),
                    panic: None,
                }
            } else {
                Seen {
                    got: vec!["Ok(())".to_owned()],
                    recorded: BTreeMap::new(),
                    marks: BTreeSet::new(),
                    panic: None,
                }
            };
            want.push((reshape, reader, seen));
        }
    }
    assert_eq!(want, got);
}
