//! The plans of the search's sequences, each made once per test binary and kept under the target
//! directory for every test of that binary.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

use adtb::testing::{Pid, Quarantine};

use super::case::{self, Plan};
use super::generate;
use super::ledger::LogWrite;
use super::pool;
use super::stated::Held;
use super::world::{Family, Ids, RELS, Rel, WriteOut};

/// The most binaries whose plans are kept: each build of the storage binary a target directory
/// runs at once, `-p adtb-sim` and the workspace's, and the ones before a rebuild.
const BINARIES: usize = 4;

/// Returns the plan of sequence `seed`, with a vacuum beside each write when `concurrent`: the one
/// this test binary kept, else one made now and kept. A process waits for another making the
/// same plan.
pub fn plan(seed: u64, concurrent: bool) -> Plan {
    let path = path(seed, concurrent);
    if let Some(plan) = read(&path, seed, concurrent) {
        return plan;
    }
    let lock = remade(|| File::create(path.with_extension("lock")))
        .unwrap_or_else(|e| panic!("the lock of {} opens: {e}", path.display()));
    lock.lock()
        .unwrap_or_else(|e| panic!("the lock of {} is taken: {e}", path.display()));
    if let Some(plan) = read(&path, seed, concurrent) {
        return plan;
    }
    let plan = case::plan(seed, generate::sequence(seed), concurrent);
    let bytes = encode(&plan);
    assert!(
        plan == decode(seed, concurrent, &bytes),
        "the plan of seed {seed} reads back as written"
    );
    let part = path.with_extension(std::process::id().to_string());
    remade(|| fs::write(&part, &bytes).and_then(|()| fs::rename(&part, &path)))
        .unwrap_or_else(|e| panic!("{} is kept: {e}", path.display()));
    plan
}

/// Returns `op`'s result, running it again in a directory made anew when another binary's run
/// removed the directory of kept plans.
fn remade<T>(op: impl Fn() -> io::Result<T>) -> io::Result<T> {
    match op() {
        Err(e) if e.kind() == ErrorKind::NotFound => {
            fs::create_dir_all(kept())?;
            op()
        }
        done => done,
    }
}

/// Returns the path of the kept plan of `seed` and `concurrent`.
fn path(seed: u64, concurrent: bool) -> PathBuf {
    let run = if concurrent { "concurrent" } else { "serial" };
    kept().join(format!("{seed}-{run}.plan"))
}

/// Returns the plan of `seed` kept at `path`, `None` when none is.
fn read(path: &Path, seed: u64, concurrent: bool) -> Option<Plan> {
    match fs::read(path) {
        Ok(bytes) => Some(decode(seed, concurrent, &bytes)),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => panic!("{} reads: {e}", path.display()),
    }
}

/// Returns the directory of the plans this test binary keeps, named by a hash of the binary's
/// bytes, made on first use. Removes the directories of all but the [`BINARIES`] binaries whose
/// plans last changed latest, this one counted.
fn kept() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let exe = std::env::current_exe().expect("the test binary's path");
        let bytes = fs::read(&exe).expect("the test binary reads");
        let hash = blake3::hash(&bytes).to_hex();
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("damage-plans");
        let dir = root.join(&hash[..32]);
        fs::create_dir_all(&dir).expect("the directory of kept plans is made");
        let mut others: Vec<(SystemTime, PathBuf)> = fs::read_dir(&root)
            .expect("the kept plans list")
            .map(|entry| entry.expect("a kept plan's directory").path())
            .filter(|other| *other != dir)
            .filter_map(|other| Some((changed(&other)?, other)))
            .collect();
        others.sort_unstable_by(|a, b| b.cmp(a));
        for (_, other) in others.into_iter().skip(BINARIES - 1) {
            match fs::remove_dir_all(&other) {
                Err(e)
                    if !matches!(e.kind(), ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty) =>
                {
                    panic!("{} is removed: {e}", other.display())
                }
                _ => {}
            }
        }
        dir
    })
}

/// Returns when `path` last changed, `None` when it is gone.
fn changed(path: &Path) -> Option<SystemTime> {
    match fs::metadata(path).and_then(|m| m.modified()) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => panic!("{} has no time: {e}", path.display()),
    }
}

/// A value the kept plans hold, written as bytes and read back.
trait Code: Sized {
    fn put(&self, out: &mut Vec<u8>);
    fn take(input: &mut &[u8]) -> Self;
}

fn bytes<const N: usize>(input: &mut &[u8]) -> [u8; N] {
    let (head, rest) = input.split_at(N);
    *input = rest;
    head.try_into().expect("a split of N bytes")
}

impl Code for u64 {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend(self.to_le_bytes());
    }
    fn take(input: &mut &[u8]) -> Self {
        u64::from_le_bytes(bytes(input))
    }
}

impl Code for u32 {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend(self.to_le_bytes());
    }
    fn take(input: &mut &[u8]) -> Self {
        u32::from_le_bytes(bytes(input))
    }
}

impl Code for u16 {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend(self.to_le_bytes());
    }
    fn take(input: &mut &[u8]) -> Self {
        u16::from_le_bytes(bytes(input))
    }
}

impl Code for usize {
    fn put(&self, out: &mut Vec<u8>) {
        (*self as u64).put(out);
    }
    fn take(input: &mut &[u8]) -> Self {
        usize::try_from(u64::take(input)).expect("a kept count fits a usize")
    }
}

impl Code for bool {
    fn put(&self, out: &mut Vec<u8>) {
        out.push(u8::from(*self));
    }
    fn take(input: &mut &[u8]) -> Self {
        bytes::<1>(input)[0] == 1
    }
}

impl Code for String {
    fn put(&self, out: &mut Vec<u8>) {
        self.len().put(out);
        out.extend(self.as_bytes());
    }
    fn take(input: &mut &[u8]) -> Self {
        let n = usize::take(input);
        let (text, rest) = input.split_at(n);
        *input = rest;
        String::from_utf8(text.to_vec()).expect("a kept text is UTF-8")
    }
}

impl Code for Pid {
    fn put(&self, out: &mut Vec<u8>) {
        self.0.put(out);
    }
    fn take(input: &mut &[u8]) -> Self {
        Pid(u64::take(input))
    }
}

impl<T: Code> Code for Option<T> {
    fn put(&self, out: &mut Vec<u8>) {
        self.is_some().put(out);
        if let Some(v) = self {
            v.put(out);
        }
    }
    fn take(input: &mut &[u8]) -> Self {
        bool::take(input).then(|| T::take(input))
    }
}

impl<T: Code> Code for Vec<T> {
    fn put(&self, out: &mut Vec<u8>) {
        self.len().put(out);
        for v in self {
            v.put(out);
        }
    }
    fn take(input: &mut &[u8]) -> Self {
        let n = usize::take(input);
        (0..n).map(|_| T::take(input)).collect()
    }
}

impl<T: Code + Ord> Code for BTreeSet<T> {
    fn put(&self, out: &mut Vec<u8>) {
        self.len().put(out);
        for v in self {
            v.put(out);
        }
    }
    fn take(input: &mut &[u8]) -> Self {
        let n = usize::take(input);
        (0..n).map(|_| T::take(input)).collect()
    }
}

impl<K: Code + Ord, V: Code> Code for BTreeMap<K, V> {
    fn put(&self, out: &mut Vec<u8>) {
        self.len().put(out);
        for (k, v) in self {
            k.put(out);
            v.put(out);
        }
    }
    fn take(input: &mut &[u8]) -> Self {
        let n = usize::take(input);
        (0..n).map(|_| (K::take(input), V::take(input))).collect()
    }
}

impl<A: Code, B: Code> Code for (A, B) {
    fn put(&self, out: &mut Vec<u8>) {
        self.0.put(out);
        self.1.put(out);
    }
    fn take(input: &mut &[u8]) -> Self {
        let a = A::take(input);
        (a, B::take(input))
    }
}

impl<A: Code, B: Code, C: Code> Code for (A, B, C) {
    fn put(&self, out: &mut Vec<u8>) {
        self.0.put(out);
        self.1.put(out);
        self.2.put(out);
    }
    fn take(input: &mut &[u8]) -> Self {
        let a = A::take(input);
        let b = B::take(input);
        (a, b, C::take(input))
    }
}

/// Write `v` as its place in `all`.
fn put_one<T: PartialEq + std::fmt::Debug>(all: &[T], v: &T, out: &mut Vec<u8>) {
    let at = all.iter().position(|x| x == v);
    out.push(u8::try_from(at.unwrap_or_else(|| panic!("{v:?} is listed"))).expect("a short list"));
}

/// Returns the member of `all` at the place read from `input`.
fn take_one<T: Copy>(all: &[T], input: &mut &[u8]) -> T {
    all[usize::from(bytes::<1>(input)[0])]
}

impl Code for Rel {
    fn put(&self, out: &mut Vec<u8>) {
        put_one(&RELS, self, out);
    }
    fn take(input: &mut &[u8]) -> Self {
        take_one(&RELS, input)
    }
}

impl Code for Held {
    fn put(&self, out: &mut Vec<u8>) {
        put_one(&Held::ALL, self, out);
    }
    fn take(input: &mut &[u8]) -> Self {
        take_one(&Held::ALL, input)
    }
}

impl Code for Family {
    fn put(&self, out: &mut Vec<u8>) {
        put_one(&[Family::Heap, Family::Tree], self, out);
    }
    fn take(input: &mut &[u8]) -> Self {
        take_one(&[Family::Heap, Family::Tree], input)
    }
}

impl Code for Quarantine {
    fn put(&self, out: &mut Vec<u8>) {
        let (tag, id) = match *self {
            Quarantine::Database => (0_u8, 0),
            Quarantine::Heap(id) => (1, id),
            Quarantine::Index(id) => (2, id),
        };
        out.push(tag);
        id.put(out);
    }
    fn take(input: &mut &[u8]) -> Self {
        let tag = bytes::<1>(input)[0];
        let id = u32::take(input);
        match tag {
            0 => Quarantine::Database,
            1 => Quarantine::Heap(id),
            2 => Quarantine::Index(id),
            _ => panic!("a kept mark has tag {tag}, which no mark writes"),
        }
    }
}

impl Code for WriteOut {
    fn put(&self, out: &mut Vec<u8>) {
        self.op.put(out);
        self.pid.put(out);
        self.kind.put(out);
        self.owner.put(out);
        self.lsn.put(out);
        self.changed.put(out);
        self.during.put(out);
    }
    fn take(input: &mut &[u8]) -> Self {
        WriteOut {
            op: Code::take(input),
            pid: Code::take(input),
            kind: Code::take(input),
            owner: Code::take(input),
            lsn: Code::take(input),
            changed: Code::take(input),
            during: Code::take(input),
        }
    }
}

impl Code for LogWrite {
    fn put(&self, out: &mut Vec<u8>) {
        self.path.put(out);
        self.offset.put(out);
        self.lsn.put(out);
        self.submitted.put(out);
        self.outs.put(out);
        self.read_later.put(out);
        self.after.put(out);
        self.ended.put(out);
        self.folds.put(out);
        self.planned.put(out);
        self.dropped.put(out);
    }
    fn take(input: &mut &[u8]) -> Self {
        LogWrite {
            path: Code::take(input),
            offset: Code::take(input),
            lsn: Code::take(input),
            submitted: Code::take(input),
            outs: Code::take(input),
            read_later: Code::take(input),
            after: Code::take(input),
            ended: Code::take(input),
            folds: Code::take(input),
            planned: Code::take(input),
            dropped: Code::take(input),
        }
    }
}

impl Code for Ids {
    fn put(&self, out: &mut Vec<u8>) {
        self.heaps.put(out);
        self.indexes.put(out);
        self.large.put(out);
        self.key_indexes.put(out);
        self.marks.put(out);
        self.holders.put(out);
        self.catalog_root.put(out);
        self.counters.put(out);
        self.retired.put(out);
    }
    fn take(input: &mut &[u8]) -> Self {
        Ids {
            heaps: Code::take(input),
            indexes: Code::take(input),
            large: Code::take(input),
            key_indexes: Code::take(input),
            marks: Code::take(input),
            holders: Code::take(input),
            catalog_root: Code::take(input),
            counters: Code::take(input),
            retired: Code::take(input),
        }
    }
}

fn encode(plan: &Plan) -> Vec<u8> {
    let mut out = Vec::new();
    plan.writes.put(&mut out);
    plan.log.put(&mut out);
    plan.list.put(&mut out);
    plan.ids.put(&mut out);
    plan.held.put(&mut out);
    out
}

/// Returns the plan of `seed` read from `bytes`.
///
/// Precondition: `bytes` is what [`encode`] wrote of the plan of `seed` and `concurrent` in a run
/// of this binary.
fn decode(seed: u64, concurrent: bool, bytes: &[u8]) -> Plan {
    let mut input = bytes;
    let plan = Plan {
        seed,
        ops: generate::sequence(seed),
        concurrent,
        writes: Code::take(&mut input),
        log: Code::take(&mut input),
        list: Code::take(&mut input),
        ids: Code::take(&mut input),
        held: Code::take(&mut input),
    };
    assert!(
        input.is_empty(),
        "the kept plan of seed {seed} ends where its last part does"
    );
    plan
}

#[test]
fn a_kept_plan_equals_the_plan_made_fresh() {
    let runs = [(1, false), (generate::STEERED, false), (1, true)];
    pool::map(
        &runs,
        |(seed, concurrent)| format!("seed {seed} concurrent {concurrent}"),
        |&(seed, concurrent)| {
            drop(plan(seed, concurrent));
            let kept = read(&path(seed, concurrent), seed, concurrent);
            let fresh = case::plan(seed, generate::sequence(seed), concurrent);
            assert!(
                kept.is_some_and(|k| k == fresh),
                "the kept plan of seed {seed}, concurrent {concurrent}, is the plan made fresh"
            );
        },
    );
}
