//! The database a case runs on, the operations a sequence is made of, and the run of a sequence
//! across the processes its reopens, crashes and migration divide it into.

use adtb::testing::{Damage, Failed, Knobs, Object, PageKind, Pid, Plant, Quarantine, page};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adtb::runtime::{OpKind, Runtime, Threads};
use adtb::{
    CreateOptions, Database, Error, FatalReason, Log, OpenError, OpenOptions, TxnCallOptions,
    TxnOptions,
};
use adtb_sim::{Config, Match, OpLog, Outcome, Planned, Sim, SimRuntime, Submitted};
use adtb_testkit::damage::DataFile;
use adtb_testkit::search::Files;
use adtb_testkit::{common, crash};

use super::ledger::{self, Evidence, LogWrite};
use super::stated::{self, Class, Held, Holding, Phase};

pub const FILE: &str = "search.adt";
pub const PAGE: usize = 8192;
pub const DATA: DataFile = DataFile::new(FILE, PAGE);
pub const SEGMENT: u64 = 64 * 1024;
/// The bytes the simulated device writes atomically.
pub const UNIT: usize = 512;
const _: () = assert!(PAGE / UNIT == u16::BITS as usize);

const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Doc = { id: Key docs, body: Text }
add relation docs : Doc key id generate sequence
add type Scrap = { id: Key scraps, note: Text }
add relation scraps : Scrap key id generate sequence
";

const TAGS: &str = r"module app
add type Label = { id: Key tags, label: Text }
add relation tags : Label key id generate sequence
";

const UNTAG: &str = "module app\ndrop relation tags\ndrop type Label\n";

/// The highest schema version past the first: the one that adds `tags`, then the one that drops
/// it.
pub const LAST: u8 = 2;

/// The relations a sequence writes, by the name a text gives them. The model holds the rows of
/// each but `scraps`, whose page [`Op::Damage`] damages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rel {
    Users,
    Docs,
    Tags,
    Scraps,
}

pub const RELS: [Rel; 4] = [Rel::Users, Rel::Docs, Rel::Tags, Rel::Scraps];

impl Rel {
    pub fn name(self) -> &'static str {
        match self {
            Rel::Users => "users",
            Rel::Docs => "docs",
            Rel::Tags => "tags",
            Rel::Scraps => "scraps",
        }
    }

    pub fn qualified(self) -> String {
        format!("app::{}", self.name())
    }

    pub fn ctor(self) -> &'static str {
        match self {
            Rel::Users => "User",
            Rel::Docs => "Doc",
            Rel::Tags => "Label",
            Rel::Scraps => "Scrap",
        }
    }

    fn field(self) -> &'static str {
        match self {
            Rel::Users => "name",
            Rel::Docs => "body",
            Rel::Tags => "label",
            Rel::Scraps => "note",
        }
    }

    /// Returns the text reading every row's value.
    pub fn read(self) -> String {
        format!("from r in {} select r.{}", self.name(), self.field())
    }

    /// Returns the text reading the value of the row with key `id`.
    pub fn lookup(self, id: i64) -> String {
        let n = self.name();
        format!(
            "from r in {n} where r.id == {n}.key({id}) select r.{}",
            self.field()
        )
    }
}

/// Returns the value row `id` of `rel` holds at its version `v`. A document's body is larger than
/// a heap page and lives in the large-value store.
pub fn value(rel: Rel, id: i64, v: u32) -> String {
    let head = format!("{}{id}v{v}", rel.name());
    match rel {
        Rel::Docs => format!("{head}{}", "x".repeat(3 * PAGE)),
        Rel::Users | Rel::Tags | Rel::Scraps => format!("{head}{}", "y".repeat(400)),
    }
}

/// One step of a sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// Insert rows `ids` of `rel` in one transaction.
    Insert(Rel, Vec<i64>),
    /// Give rows `ids` of `rel` a new version in one transaction.
    Update(Rel, Vec<i64>),
    /// Delete rows `ids` of `rel` in one transaction.
    Delete(Rel, Vec<i64>),
    /// Vacuum `rel`, freeing the pages its deletes emptied.
    Vacuum(Rel),
    /// Write every changed page and take a checkpoint.
    Checkpoint,
    /// Recycle the log within the session, after changing rows of `users` until it has grown by
    /// three segments.
    Recycle,
    /// Close, then open with the next migration: the one that adds `tags`, then the one that
    /// drops it and frees its pages.
    Migrate,
    /// Close and open again.
    Reopen,
    /// Crash and open again.
    Crash,
    /// Insert a row of `scraps` and record its heap page damaged, as a read that found it so
    /// does.
    Damage,
}

/// The value of each row of each relation, from the operations the test ran.
pub type Rows = BTreeMap<Rel, BTreeMap<i64, u32>>;

/// What the run of a sequence acknowledged: the rows after each commit that returned, in order,
/// from the rows before the first.
#[derive(Clone, Debug, Default)]
pub struct Acked {
    pub states: Vec<Rows>,
    /// The writes of the run's log of writes submitted before each state after the first was
    /// acknowledged.
    pub submitted: Vec<usize>,
    /// The migrations past the first an open applied and acknowledged.
    pub version: u8,
}

impl Acked {
    pub fn last(&self) -> &Rows {
        self.states.last().expect("the empty state is the first")
    }

    fn push(&mut self, rows: Rows, submitted: usize) {
        self.states.push(rows);
        self.submitted.push(submitted);
    }
}

/// Returns `rows` after `op` commits.
pub fn applied(mut rows: Rows, op: &Op) -> Rows {
    match op {
        Op::Insert(rel, ids) => {
            let of = rows.entry(*rel).or_default();
            for &id in ids {
                of.insert(id, 0);
            }
        }
        Op::Update(rel, ids) => {
            let of = rows.entry(*rel).or_default();
            for id in ids {
                if let Some(v) = of.get_mut(id) {
                    *v += 1;
                }
            }
        }
        Op::Delete(rel, ids) => {
            let of = rows.entry(*rel).or_default();
            for id in ids {
                of.remove(id);
            }
        }
        Op::Migrate => {
            rows.remove(&Rel::Tags);
        }
        _ => {}
    }
    rows
}

/// Returns the text of the statement `op` runs for row `id`, at the version the row has in `rows`
/// after the statement.
fn statement(op: &Op, id: i64, after: &Rows) -> String {
    let (rel, v) = match op {
        Op::Insert(rel, _) | Op::Update(rel, _) | Op::Delete(rel, _) => {
            (*rel, after.get(rel).and_then(|r| r.get(&id)).copied())
        }
        _ => unreachable!("only a write runs statements"),
    };
    let (n, c, f) = (rel.name(), rel.ctor(), rel.field());
    let key = format!("{n}.key({id})");
    match op {
        Op::Insert(..) => format!(
            r#"insert {n} {{ id: {key}, {f}: "{}" }}"#,
            value(rel, id, 0)
        ),
        Op::Update(..) => {
            let v = v.unwrap_or(0);
            format!(
                r#"update r in {n} where key r == {key} set {c} {{ r with {f}: "{}" }}"#,
                value(rel, id, v)
            )
        }
        _ => format!("delete r in {n} where key r == {key}"),
    }
}

/// One page of a write to the data file, as the device saw the write submitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteOut {
    /// The number of the write among the writes of whole pages to the data file.
    pub op: u64,
    pub pid: Pid,
    pub kind: u16,
    pub owner: u32,
    pub lsn: u64,
    /// The units of [`UNIT`] bytes in which the page differs from what the last earlier
    /// write-out of it wrote, zeros before the first: bit `u` for unit `u`.
    pub changed: u16,
    pub during: During,
}

/// The operation running when a write-out was submitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum During {
    /// The create of the file.
    Creating,
    /// The operation of this index in the sequence: the sequence's length for the close that ends
    /// it.
    Op(usize),
}

/// The pages of the writes a run numbered, in order.
#[derive(Clone, Debug, Default)]
pub struct Trace {
    pub writes: Vec<WriteOut>,
    /// The image each write-out of a commit log page wrote, by its index in `writes`.
    pub images: BTreeMap<usize, Vec<u8>>,
}

/// Returns the bytes the data file of `files` holds for page `pid`, zeros past its end.
pub fn page(files: &Files, pid: Pid) -> Vec<u8> {
    let data = DATA.bytes(files);
    let range = DATA.range(pid);
    let mut out = vec![0; PAGE];
    if range.start < data.len() {
        let end = range.end.min(data.len());
        out[..end - range.start].copy_from_slice(&data[range.start..end]);
    }
    out
}

/// Returns `ops` limited to those on the data file, under its name or as its create builds it.
pub fn on_data(ops: Match) -> Match {
    ops.prefixed(FILE).except(Match::any().containing("-"))
}

/// Returns the match of the writes of whole pages to the data file.
pub fn write_outs() -> Match {
    on_data(Match::kind(OpKind::Write)).whole(PAGE as u64)
}

/// Returns the pages of `writes`, the operation each ran during given by `during`: the index in
/// `writes` from which each operation ran.
#[expect(
    clippy::disallowed_methods,
    reason = "reads owners from file images, which have no guard"
)]
fn trace(writes: &OpLog, during: &[(usize, usize)]) -> Trace {
    let mut trace = Trace::default();
    let mut last: BTreeMap<Pid, &[u8]> = BTreeMap::new();
    let zeros = [0; PAGE];
    let writes = writes.entries();
    for (number, write) in writes.iter().enumerate() {
        let during = during
            .iter()
            .rev()
            .find(|(from, _)| *from <= number)
            .map_or(During::Creating, |(_, i)| During::Op(*i));
        let offset = write.offset().unwrap();
        for (i, page) in write.bytes().unwrap().chunks(PAGE).enumerate() {
            let pid = Pid(offset / PAGE as u64 + i as u64);
            let before = last.insert(pid, page).unwrap_or(&zeros);
            let changed = page
                .chunks(UNIT)
                .zip(before.chunks(UNIT))
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .fold(0, |mask, (u, _)| mask | (1 << u));
            if page::kind(page) == PageKind::CommitLog as u16 {
                trace.images.insert(trace.writes.len(), page.to_vec());
            }
            trace.writes.push(WriteOut {
                op: number as u64,
                pid,
                kind: page::kind(page),
                owner: page::owner(page),
                lsn: page::lsn(page).0,
                changed,
                during,
            });
        }
    }
    trace
}

pub fn options() -> OpenOptions {
    OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(2),
        pool_bytes: Some(48 * PAGE as u64),
        testing: Knobs::without_background_vacuum().with_segment_size(SEGMENT),
        ..OpenOptions::default()
    }
}

/// Returns the log of the migrations up to `version` past the first.
pub fn log(version: u8) -> Log {
    Log::new([APP, TAGS, UNTAG].into_iter().take(1 + version as usize))
}

/// Returns the version of the file's own log when `e` is a mark refusing the migration past it:
/// the whole database's, or that of a constructor relation the migration reads.
pub fn refused_marked(e: &OpenError) -> Option<u8> {
    match e {
        OpenError::MigrationRefused {
            version,
            reason:
                Error::Fatal {
                    reason: FatalReason::Quarantined { .. },
                },
        } => u8::try_from(version.checked_sub(2)?).ok(),
        _ => None,
    }
}

/// Open `FILE` with the log to `version`, and when a mark refuses a migration past the file's,
/// with the file's own log. Under `retry`, do so once more when that open fails,
/// as an operator would after a failure that may have cleared. Returns the open and the version of
/// its log.
pub fn open_own<R: Runtime>(
    rt: &R,
    version: u8,
    options: &OpenOptions,
    retry: bool,
) -> (Result<Database<R>, OpenError>, u8) {
    let at = |v: u8| Database::open_in(rt.clone(), FILE, &log(v), options);
    let own = |v: u8| {
        let open = at(v);
        match open.as_ref().err().and_then(refused_marked) {
            Some(own) if own < v => (at(own), own),
            _ => (open, v),
        }
    };
    let (open, version) = own(version);
    if retry && open.is_err() {
        return own(version);
    }
    (open, version)
}

/// Returns the commit's result of the transaction running `op` on `db`, with `rows` the
/// acknowledged rows before it.
fn write<R: Runtime>(db: &Database<R>, op: &Op, rows: &Rows) -> Result<(), Error> {
    let after = applied(rows.clone(), op);
    let (Op::Insert(_, ids) | Op::Update(_, ids) | Op::Delete(_, ids)) = op else {
        unreachable!("only a write runs statements")
    };
    let mut t = db.begin(&TxnOptions::default())?;
    for &id in ids {
        t.query_text(&statement(op, id, &after), &[], &TxnCallOptions::default())?;
    }
    t.commit(None).map(drop)
}

/// Where a page is placed: its holder; the user relation it belongs to for a page of a
/// relation's heap, history heap, key index or history index; and the owner the pages of the
/// listed structure carry, for a structure listed by its object.
pub type Placed = (Holding, Option<Rel>, Option<u32>);

/// Where each allocated page was placed after an operation, by the structure listings of the
/// observing run; free pages are absent.
pub type Holdings = BTreeMap<Pid, Placed>;

/// Where a free page is placed.
pub const FREE: Placed = ((Held::Free, None), None, None);

/// The owner each user relation's pages carry: its heap's and its history heap's, which their
/// directory and visibility map pages carry too, its key index's and history index's, and its
/// large-value store's; the marks a page of each takes; and the commit log's status pages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ids {
    pub heaps: BTreeMap<u32, Rel>,
    pub indexes: BTreeMap<u32, Rel>,
    pub large: BTreeSet<u32>,
    /// The owner of each relation's key index, which a read by key goes through.
    pub key_indexes: BTreeMap<Rel, u32>,
    /// The mark of each relation's heap and of its key index.
    pub marks: BTreeMap<Rel, (Quarantine, Quarantine)>,
    /// Every status page of the commit log while the clean run was observed.
    pub status: BTreeSet<Pid>,
}

/// Returns whether the schema `db` read holds the change of migration `version`: `users`, and
/// `tags` exactly when that version has it.
fn applies<R: Runtime>(db: &Database<R>, version: u8) -> bool {
    let s = db.schema();
    let app: BTreeSet<&str> = s
        .relations
        .iter()
        .filter(|r| s.module(r.module).name == "app")
        .map(|r| r.name.as_ref())
        .collect();
    app.contains(Rel::Users.name()) && app.contains(Rel::Tags.name()) == present(Rel::Tags, version)
}

/// Returns whether `db` opened without reading its catalog: `verify` of a relation then fails
/// with the whole-database quarantine.
fn unread<R: Runtime>(db: &Database<R>) -> bool {
    let users = Rel::Users.qualified();
    match db.verify(Some(&users)).map(drop) {
        Err(Error::Fatal {
            reason: FatalReason::Quarantined { what },
        }) => what == "the database",
        _ => false,
    }
}

/// Returns whether the page of marks `db` writes holds the whole database's mark.
fn durably_whole<R: Runtime>(db: &Database<R>) -> bool {
    let t = db.testing();
    t.probe().marked(Quarantine::Database)
        && t.probe()
            .page(t.root(Object::Marks).unwrap())
            .is_ok_and(|page| {
                ledger::marks(&page).is_some_and(|m| m.contains(&Quarantine::Database))
            })
}

/// Returns whether `db` holds a mark on the whole database, under which an open applies no
/// migration.
fn marked_whole<R: Runtime>(db: &Database<R>) -> bool {
    db.testing().probe().marks().contains(&Quarantine::Database)
}

/// Returns whether `rel` exists at schema `version`.
pub fn present(rel: Rel, version: u8) -> bool {
    rel != Rel::Tags || version == 1
}

/// Returns where `db` places each allocated page: the pages a swap retired, then the pages of
/// each user relation present at schema `version`, then those of every other structure by its
/// listing, then the pages placed with no walk, and every other allocated page unreached.
fn holdings<R: Runtime>(db: &Database<R>, version: u8) -> Holdings {
    let t = db.testing();
    let mut out = Holdings::new();
    let mut place = |pages: Vec<Pid>, holding: Holding, rel: Option<Rel>, owner: Option<u32>| {
        for p in pages {
            out.entry(p).or_insert((holding, rel, owner));
        }
    };
    place(t.retired(), (Held::Retired, None), None, None);
    let owner = |o: Object| Some(t.owner(o).expect("a listed object's owner"));
    let held = |o: Object| stated::holding(t.holder(o).expect("a listed object's holder"));
    let listed = |o: Object| match o {
        Object::Index(_) | Object::HistoryIndex(_) | Object::CommitTimes => t.pages(o),
        _ => t.owned(o),
    };
    for rel in RELS.into_iter().filter(|r| present(*r, version)) {
        let q = rel.qualified();
        let heap = t.heap(&q, rel.ctor()).expect("the relation's heap");
        let index = t.index(&q, "key id").expect("the relation's key index");
        let history = [t.history(&q, rel.ctor()), t.history_index(&q)];
        for o in [heap, index]
            .into_iter()
            .chain(history.into_iter().flatten())
        {
            place(
                listed(o).expect("the relation's pages"),
                held(o),
                Some(rel),
                owner(o),
            );
        }
    }
    for o in t.objects().into_iter().chain([Object::CatalogRows]) {
        place(
            listed(o).expect("a structure's pages"),
            held(o),
            None,
            owner(o),
        );
    }
    if let Ok(pages) = listed(Object::CommitTimes) {
        place(
            pages,
            held(Object::CommitTimes),
            None,
            owner(Object::CommitTimes),
        );
    }
    let log = t.commit_log();
    let mut clog = vec![log.root()];
    clog.extend(log.directory());
    clog.extend(log.status().values());
    place(clog, (Held::CommitLog, None), None, None);
    for (o, h) in [
        (Object::Catalog, Held::CatalogRoot),
        (Object::Sequences, Held::Counters),
        (Object::Marks, Held::MarksPage),
        (Object::DamageList, Held::DamageList),
    ] {
        place(
            vec![t.root(o).expect("an engine page")],
            (h, None),
            None,
            None,
        );
    }
    for (p, holder) in t.unwalked() {
        place(vec![p], stated::holding(holder), None, None);
    }
    let allocated = t.allocated().expect("the allocated pages");
    place(allocated, (Held::Unreached, None), None, None);
    out
}

/// Add the owners of the relations of `db` at schema `version` to `out`.
fn ids<R: Runtime>(db: &Database<R>, version: u8, out: &mut Ids) {
    let t = db.testing();
    for rel in RELS.into_iter().filter(|r| present(*r, version)) {
        let q = rel.qualified();
        let heap = t.heap(&q, rel.ctor()).expect("the relation's heap");
        let index = t.index(&q, "key id").expect("the relation's key index");
        let large = t.large(&q, rel.ctor()).expect("the relation's store");
        out.heaps.insert(t.owner(heap).unwrap(), rel);
        out.indexes.insert(t.owner(index).unwrap(), rel);
        out.large.insert(t.owner(large).unwrap());
        out.key_indexes.insert(rel, t.owner(index).unwrap());
        let own = |o| {
            let holding = stated::holding(t.holder(o).unwrap());
            assert_eq!(
                Some(t.own_mark(o).unwrap()),
                holding.1,
                "{o:?} is held by its own heap or index"
            );
            stated::durable(Class::Damaged, holding, Phase::Running)
                .expect("a page of a user relation marks it")
        };
        out.marks.insert(rel, (own(heap), own(index)));
        if let Some(h) = t.history(&q, rel.ctor()) {
            out.heaps.insert(t.owner(h).unwrap(), rel);
        }
        if let Some(ix) = t.history_index(&q) {
            out.indexes.insert(t.owner(ix).unwrap(), rel);
        }
    }
    out.status.extend(t.commit_log().status().values());
}

/// What a run of a sequence observed: what it acknowledged, the write-outs it numbered and the
/// relation holding each page after each operation, and how the run ended.
#[derive(Debug)]
pub struct Ran {
    pub acked: Acked,
    /// The writes of the run's log of writes and the write-outs submitted when each process
    /// began.
    pub processes: Vec<(usize, u64)>,
    /// Whether a planned failure of a read of the data file or the log fired.
    pub read_failed: bool,
    pub trace: Trace,
    pub held: Vec<Holdings>,
    /// The first id not reserved and the commit log's truncation bound after each operation, by
    /// the observing run.
    pub clog: Vec<(u64, u64)>,
    /// Each open of the run: the write-outs submitted before it, the operation after which its
    /// commit log stands as it found it, and the operation its write-outs count as during.
    pub opens: Vec<(u64, usize, usize)>,
    pub ids: Ids,
    /// Each operation's error, by its index, for one that failed.
    pub failed: BTreeMap<usize, String>,
    /// The index of the operation a process ended in before the sequence's end, by a crash a
    /// fault caused or an open that failed.
    pub cut: Option<usize>,
    /// The write a crash cut before its commit returned.
    pub pending: Option<Op>,
    /// The writes a vacuum on a second thread ran during.
    pub overlaps: usize,
    /// The page the sequence's damage operation damaged on purpose.
    pub damaged: Option<Pid>,
    /// The migrations, by their operation's index, whose open returned Ok under a whole-database
    /// mark without applying them.
    pub under_mark: BTreeSet<usize>,
    /// The migrations, by their operation's index, applied by an open of a file whose
    /// whole-database mark was durable.
    pub applied_under_mark: BTreeSet<usize>,
    /// The migrations, by their operation's index, whose open a mark refused.
    pub refused_under_mark: BTreeSet<usize>,
    /// The migrations a mark other than the whole database's refused, by their operation's
    /// index: the version refused and the marks the file held.
    pub refused_by_relation: BTreeMap<usize, (u8, BTreeSet<Quarantine>)>,
    /// The pages the damage records named after any open or operation of the run.
    pub recorded: BTreeSet<Pid>,
    /// The reads of the data file a plan failed, by process.
    pub failed_reads: Vec<usize>,
    /// What the file held after each process, when the run watched it.
    pub evidence: Vec<Evidence>,
    /// Every write to the log, in order.
    pub log: Vec<LogWrite>,
    pub files: Files,
}

/// The faults a run plans at numbered write-outs, set in its first process and kept by the
/// simulation for the later ones.
pub type Arm<'a> = &'a dyn Fn(&SimRuntime);

/// The state one run carries across its processes.
struct Runner<'a> {
    ops: &'a [Op],
    writes: OpLog,
    /// The writes to the log, the write-outs and the reads of the file and the log, in submission
    /// order.
    logged: OpLog,
    /// The length of `writes` when each operation began, with its index.
    during: Vec<(usize, usize)>,
    acked: Acked,
    held: Vec<Holdings>,
    clog: Vec<(u64, u64)>,
    opens: Vec<(u64, usize, usize)>,
    ids: Ids,
    failed: BTreeMap<usize, String>,
    pending: Option<Op>,
    next: usize,
    observe: bool,
    concurrent: bool,
    overlaps: usize,
    damaged: Option<Pid>,
    under_mark: BTreeSet<usize>,
    applied_under_mark: BTreeSet<usize>,
    refused_under_mark: BTreeSet<usize>,
    refused_by_relation: BTreeMap<usize, (u8, BTreeSet<Quarantine>)>,
    recorded: BTreeSet<Pid>,
}

impl Runner<'_> {
    fn note_records<R: Runtime>(&mut self, db: &Database<R>) {
        let records = db.testing().probe().records();
        self.recorded.extend(records.pages().keys().copied());
    }

    /// Returns the database `opened` opened and the version of its log, recording its failure
    /// under operation `i`.
    fn failing<R: Runtime>(
        &mut self,
        opened: (Result<Database<R>, OpenError>, u8),
        i: usize,
    ) -> Option<(Database<R>, u8)> {
        match opened {
            (Ok(db), version) => Some((db, version)),
            (Err(e), _) => {
                self.failed.insert(i, format!("open: {e:?}"));
                None
            }
        }
    }

    /// Record that operation `i`'s open was refused naming `version` as the file's, which the
    /// acknowledged operations do not leave it at.
    fn refused_naming(&mut self, i: usize, version: u8) {
        let refused = format!("refused naming version {version} as the file's");
        self.failed.insert(i, refused);
    }

    fn commit<R: Runtime>(&mut self, db: &Database<R>, op: &Op, i: usize) -> bool {
        self.pending = Some(op.clone());
        let done = write(db, op, self.acked.last());
        self.pending = None;
        match done {
            Ok(()) => {
                let rows = applied(self.acked.last().clone(), op);
                self.acked.push(rows, self.logged.len());
                true
            }
            Err(e) => {
                self.failed.insert(i, format!("{e:?}"));
                false
            }
        }
    }

    /// Run the operations from `next` up to the next crash or the end on one process. Returns
    /// whether every one ran.
    fn process(&mut self, rt: &SimRuntime) -> bool {
        let crashed = self.next.saturating_sub(1);
        self.opens
            .push((self.writes.len() as u64, crashed, crashed));
        let opened = open_own(rt, self.acked.version, &options(), true);
        let Some((mut db, at)) = self.failing(opened, self.next.saturating_sub(1)) else {
            return false;
        };
        if at != self.acked.version {
            self.refused_naming(self.next.saturating_sub(1), at);
            return false;
        }
        if self.next == 0 && self.during.is_empty() {
            let keep = "set setting RetentionWindow = { length: 1h }";
            db.query_text(keep, &[], &adtb::CallOptions::default())
                .expect("a new file keeps history");
        }
        self.note_records(&db);
        while self.next < self.ops.len() {
            let i = self.next;
            let op = &self.ops[i];
            self.during.push((self.writes.len(), i));
            self.next += 1;
            match op {
                Op::Insert(rel, _) | Op::Update(rel, _) | Op::Delete(rel, _) if self.concurrent => {
                    self.beside_vacuum(rt, &db, op, *rel, i);
                }
                Op::Insert(..) | Op::Update(..) | Op::Delete(..) => {
                    self.commit(&db, op, i);
                }
                Op::Vacuum(rel) => {
                    let text = format!("vacuum {}", rel.name());
                    if let Err(e) = db.query_text(&text, &[], &adtb::CallOptions::default()) {
                        self.failed.insert(i, format!("{e:?}"));
                    }
                }
                Op::Checkpoint => {
                    db.testing().flush();
                    db.testing().checkpoint();
                }
                Op::Recycle => self.recycle(&db, i),
                Op::Damage => self.refuse_scraps(&db, i),
                Op::Reopen | Op::Migrate => {
                    let durable = durably_whole(&db);
                    if let Err(e) = db.close() {
                        self.failed.insert(i, format!("close: {e:?}"));
                    }
                    let version = if *op == Op::Migrate {
                        (self.acked.version + 1).min(LAST)
                    } else {
                        self.acked.version
                    };
                    self.opens
                        .push((self.writes.len() as u64, i.saturating_sub(1), i));
                    let opened = open_own(rt, version, &options(), true);
                    let Some((again, opened_at)) = self.failing(opened, i) else {
                        return false;
                    };
                    db = again;
                    if opened_at != version && opened_at != self.acked.version {
                        self.refused_naming(i, opened_at);
                        return false;
                    }
                    if opened_at != version {
                        self.refused_under_mark.insert(i);
                        if !marked_whole(&db) {
                            let marks = db.testing().probe().marks();
                            self.refused_by_relation.insert(i, (version, marks));
                        }
                    }
                    let version = opened_at;
                    let marked = marked_whole(&db);
                    let migrated = version != self.acked.version && applies(&db, version);
                    if version != self.acked.version && marked && !migrated && !unread(&db) {
                        self.under_mark.insert(i);
                    }
                    if durable && migrated {
                        self.applied_under_mark.insert(i);
                    }
                    if version != self.acked.version && (migrated || !marked) {
                        self.acked.version = version;
                        let rows = applied(self.acked.last().clone(), op);
                        self.acked.push(rows, self.logged.len());
                    }
                }
                Op::Crash => {}
            }
            self.note_records(&db);
            if self.observe {
                self.held.push(holdings(&db, self.acked.version));
                let log = db.testing().commit_log();
                self.clog.push((log.reserved().0, log.bound().0));
                ids(&db, self.acked.version, &mut self.ids);
            }
            if *op == Op::Crash {
                return true;
            }
        }
        self.during.push((self.writes.len(), self.ops.len()));
        if let Err(e) = db.close() {
            self.failed.insert(self.ops.len(), format!("close: {e:?}"));
        }
        true
    }

    /// Commit `op` while a second thread vacuums `rel`, and count the commits the vacuum ran
    /// during.
    fn beside_vacuum(
        &mut self,
        rt: &SimRuntime,
        db: &Database<SimRuntime>,
        op: &Op,
        rel: Rel,
        i: usize,
    ) {
        let state = Arc::new(AtomicU8::new(0));
        let failure = Arc::new(Mutex::new(None));
        let (db2, state2, failure2) = (db.clone(), state.clone(), failure.clone());
        let thread = rt.spawn("vacuum", move || {
            state2.store(1, Ordering::SeqCst);
            let text = format!("vacuum {}", rel.name());
            if let Err(e) = db2.query_text(&text, &[], &adtb::CallOptions::default()) {
                *failure2.lock().unwrap() = Some(format!("vacuum: {e:?}"));
            }
            state2.store(2, Ordering::SeqCst);
        });
        let before = state.load(Ordering::SeqCst);
        self.commit(db, op, i);
        let after = state.load(Ordering::SeqCst);
        rt.join(thread);
        if before < 2 && after >= 1 {
            self.overlaps += 1;
        }
        if let Some(e) = failure.lock().unwrap().take() {
            self.failed.insert(i, e);
        }
    }

    fn refuse_scraps<R: Runtime>(&mut self, db: &Database<R>, i: usize) {
        let note = value(Rel::Scraps, i as i64, 0);
        let text = format!(r#"insert scraps {{ id: scraps.key({i}), note: "{note}" }}"#);
        if let Err(e) = db.query_text(&text, &[], &adtb::CallOptions::default()) {
            self.failed.insert(i, format!("{e:?}"));
            return;
        }
        let t = db.testing();
        let heap = t.heap(&Rel::Scraps.qualified(), Rel::Scraps.ctor());
        match t.pages(heap.expect("the heap of scraps")) {
            Ok(pages) => {
                self.damaged = Some(pages[0]);
                db.testing()
                    .plant(Plant::Refuse {
                        page: pages[0],
                        damage: Damage::Checksum,
                    })
                    .unwrap();
            }
            Err(Failed::Page(e)) => {
                self.failed.insert(i, format!("{e:?}"));
            }
            Err(other) => panic!("the heap of scraps lists its pages or fails on one: {other:?}"),
        }
    }

    /// Change rows of `users` until the log has grown by three segments, then recycle it.
    fn recycle<R: Runtime>(&mut self, db: &Database<R>, i: usize) {
        let t = db.testing();
        let start = t.probe().appended().0;
        while t.probe().appended().0 < start + 3 * SEGMENT {
            let users = self.acked.last().get(&Rel::Users);
            let ids: Vec<i64> = users
                .map(|r| r.keys().copied().take(4).collect())
                .unwrap_or_default();
            let op = if ids.is_empty() {
                Op::Insert(Rel::Users, vec![1000])
            } else {
                Op::Update(Rel::Users, ids)
            };
            if !self.commit(db, &op, i) {
                return;
            }
        }
        t.recycle_log();
    }
}

/// Run `ops` from a new file on a simulation seeded `seed`, with `arm` planning the faults every
/// process shares, observing the holdings after each operation when `observe`,
/// vacuuming a write's relation on a second thread during the write when `concurrent`, and
/// reading the file's evidence after each process when `watched`.
pub fn run(
    seed: u64,
    ops: &[Op],
    arm: Arm<'_>,
    observe: bool,
    concurrent: bool,
    watched: bool,
) -> Ran {
    let mut sim = Sim::new(seed, Config::default());
    let log = format!("{FILE}-log/");
    let logged = sim.logged(Match::any_of([
        write_outs(),
        Match::kind(OpKind::Write).prefixed(&log),
        on_data(Match::kind(OpKind::Read)),
        Match::kind(OpKind::Read).prefixed(&log),
    ]));
    let mut evidence = Vec::new();
    let mut runner = Runner {
        ops,
        writes: sim.logged(write_outs()),
        logged: logged.clone(),
        during: Vec::new(),
        acked: Acked {
            states: vec![Rows::new()],
            submitted: Vec::new(),
            version: 0,
        },
        held: Vec::new(),
        clog: Vec::new(),
        opens: Vec::new(),
        ids: Ids::default(),
        failed: BTreeMap::new(),
        pending: None,
        next: 0,
        observe,
        concurrent,
        overlaps: 0,
        damaged: None,
        under_mark: BTreeSet::new(),
        applied_under_mark: BTreeSet::new(),
        refused_under_mark: BTreeSet::new(),
        refused_by_relation: BTreeMap::new(),
        recorded: BTreeSet::new(),
    };
    let reads = sim.logged(on_data(Match::kind(OpKind::Read)));
    let mut starts = Vec::new();
    let mut processes = Vec::new();
    let mut armed = false;
    let mut cut = None;
    crash::silence_fatal_panics();
    loop {
        let crashing = ops[runner.next..].contains(&Op::Crash);
        if crashing {
            sim.crash_at(sim.now() + Duration::from_secs(500));
        }
        let mut whole = false;
        starts.push(reads.len());
        processes.push((logged.len(), runner.writes.len() as u64));
        let outcome = crash::run_process(
            &mut sim,
            Box::new(|rt| {
                if !armed {
                    rt.arm(write_outs());
                    arm(&rt);
                    armed = true;
                }
                whole = runner.process(&rt);
                if whole && runner.next > 0 && ops[runner.next - 1] == Op::Crash {
                    let mut io = rt.io();
                    common::sleep(&rt, &mut io, Duration::from_secs(1000));
                    unreachable!("the crash comes first");
                }
            }),
        );
        let planned = whole && runner.next > 0 && ops[runner.next - 1] == Op::Crash;
        if watched {
            evidence.extend(ledger::evidence(&sim.files()));
        }
        match outcome {
            Outcome::Finished if whole && !planned => {}
            Outcome::Crashed if planned => {}
            _ => {
                cut = Some(runner.next.saturating_sub(1));
                break;
            }
        }
        if runner.next >= ops.len() {
            break;
        }
    }
    assert_eq!(
        runner.writes.len(),
        sim.numbered().kinds().len(),
        "the trace numbers the write-outs as the plan does"
    );
    let trace = trace(&runner.writes, &runner.during);
    let files = sim.files();
    let pages = |ops: usize| trace.writes.partition_point(|w| w.op < ops as u64);
    let entries = logged.entries();
    let log = ledger::log_writes(&files, &entries, pages);
    let reads = reads.entries();
    let failed = |r: &&Submitted| matches!(r.planned(), Some(Planned::Fail(_)));
    let read_failed = entries
        .iter()
        .any(|r| r.kind() == OpKind::Read && failed(&r));
    starts.push(reads.len());
    let failed_reads = starts
        .windows(2)
        .map(|w| reads[w[0]..w[1]].iter().filter(failed).count())
        .collect();
    Ran {
        acked: runner.acked,
        processes,
        read_failed,
        trace,
        held: runner.held,
        clog: runner.clog,
        opens: runner.opens,
        ids: runner.ids,
        failed: runner.failed,
        cut,
        pending: runner.pending,
        overlaps: runner.overlaps,
        damaged: runner.damaged,
        under_mark: runner.under_mark,
        applied_under_mark: runner.applied_under_mark,
        refused_under_mark: runner.refused_under_mark,
        refused_by_relation: runner.refused_by_relation,
        recorded: runner.recorded,
        failed_reads,
        evidence,
        log,
        files,
    }
}

/// Returns the class of the page whose header names `kind` and `owner`: a heap, directory,
/// visibility map or index page of a user relation by its kind, any other page of a relation as
/// an engine page, and a page of no kind the search counts as `None`.
pub fn class(kind: u16, owner: u32, ids: &Ids) -> Option<Kind> {
    let k = PageKind::from_u16(kind)?;
    let heap = ids.heaps.contains_key(&owner);
    Some(match k {
        PageKind::HeapFixed | PageKind::HeapSlotted if ids.large.contains(&owner) => Kind::Large,
        PageKind::HeapFixed | PageKind::HeapSlotted if heap => Kind::Heap,
        PageKind::FreeSpaceMap if heap => Kind::Directory,
        PageKind::VisibilityMap if heap => Kind::VisibilityMap,
        PageKind::IndexLeaf | PageKind::IndexInner | PageKind::HashBucket
            if ids.indexes.contains_key(&owner) =>
        {
            Kind::Index
        }
        PageKind::HeapFixed
        | PageKind::HeapSlotted
        | PageKind::FreeSpaceMap
        | PageKind::VisibilityMap
        | PageKind::IndexLeaf
        | PageKind::IndexInner
        | PageKind::HashBucket
        | PageKind::Catalog => Kind::Engine,
        PageKind::CommitLog => Kind::CommitLog,
        PageKind::LsnMap => Kind::LsnMap,
        PageKind::DamageList => Kind::DamageList,
        PageKind::AllocationBitmap => Kind::Bitmap,
        PageKind::Quarantine => Kind::Marks,
    })
}

/// The kinds of page that hold data or evidence. The first four are pages of a user relation;
/// `Engine` is any page of a relation of an engine module, the catalog's included.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Heap,
    Directory,
    Index,
    VisibilityMap,
    Large,
    Engine,
    CommitLog,
    LsnMap,
    DamageList,
    Bitmap,
    Marks,
}

pub const KINDS: [Kind; 11] = adtb_testkit::every![Kind;
    Kind::Heap,
    Kind::Directory,
    Kind::Index,
    Kind::VisibilityMap,
    Kind::Large,
    Kind::Engine,
    Kind::CommitLog,
    Kind::LsnMap,
    Kind::DamageList,
    Kind::Bitmap,
    Kind::Marks,
];

impl Kind {
    /// Returns the one structure that holds every page of this kind, `None` for a kind more than
    /// one structure holds.
    pub fn sole_holder(self) -> Option<Held> {
        match self {
            Kind::Heap
            | Kind::Directory
            | Kind::Index
            | Kind::VisibilityMap
            | Kind::Large
            | Kind::Engine => None,
            Kind::CommitLog => Some(Held::CommitLog),
            Kind::LsnMap => Some(Held::LsnMap),
            Kind::DamageList => Some(Held::DamageList),
            Kind::Bitmap => Some(Held::Bitmap),
            Kind::Marks => Some(Held::MarksPage),
        }
    }

    /// Returns whether `held` is a structure listed by no object that holds pages of this kind.
    pub fn holds_unlisted(self, held: Held) -> bool {
        match self {
            Kind::Heap | Kind::Directory | Kind::Index | Kind::VisibilityMap | Kind::Large => false,
            Kind::Engine => matches!(held, Held::CatalogRoot | Held::Counters),
            Kind::CommitLog => held == Held::CommitLog,
            Kind::LsnMap => held == Held::LsnMap,
            Kind::DamageList => held == Held::DamageList,
            Kind::Bitmap => held == Held::Bitmap,
            Kind::Marks => held == Held::MarksPage,
        }
    }
}
