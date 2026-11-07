//! The database a case runs on, the operations a sequence is made of, and the run of a sequence
//! across the processes its reopens, crashes and migration divide it into.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adtb::runtime::{Op as IoOp, Runtime, Threads};
use adtb::storage::{self, Damage, PageKind, Pid, Quarantine};
use adtb::{CreateOptions, Database, Error, Log, OpenOptions, TxnCallOptions, TxnOptions};
use adtb_sim::{Config, Outcome, Sim};
use adtb_simkit::search::{Files, Hooked};
use adtb_simkit::{common, crash};

pub const FILE: &str = "search.adt";
pub const PAGE: usize = 8192;
pub const SEGMENT: u64 = 64 * 1024;

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
    /// The migrations past the first an open applied and acknowledged.
    pub version: u8,
}

impl Acked {
    pub fn last(&self) -> &Rows {
        self.states.last().expect("the empty state is the first")
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
    /// The index in the sequence of the operation running when it was submitted: the sequence's
    /// length for the close that ends it, [`CREATING`] for the create.
    pub during: usize,
}

/// The pages of the writes a run numbered, in order, with the operation running and a count
/// of the writes so far.
#[derive(Clone, Debug, Default)]
pub struct Trace {
    pub writes: Vec<WriteOut>,
    pub during: usize,
    pub ops: u64,
}

pub type Traced = Arc<Mutex<Trace>>;

/// The operation index of the write-outs of the create.
pub const CREATING: usize = usize::MAX;

/// Returns the bytes the data file of `files` holds for page `pid`, zeros past its end.
pub fn page(files: &Files, pid: Pid) -> Vec<u8> {
    let data = &files[FILE];
    let at = pid.0 as usize * PAGE;
    let mut out = vec![0; PAGE];
    if at < data.len() {
        let end = (at + PAGE).min(data.len());
        out[..end - at].copy_from_slice(&data[at..end]);
    }
    out
}

/// Returns whether `path` names the data file, under its own name or the name it is created at.
pub fn is_data(path: Option<&str>) -> bool {
    path.is_some_and(|p| {
        p == FILE
            || p.strip_prefix(FILE)
                .and_then(|rest| rest.strip_prefix(".creating/"))
                .is_some_and(|name| !name.contains('/') && !name.contains('-'))
    })
}

/// Returns whether `op` is a write of whole pages to the data file, recording it in `trace` when
/// it is.
pub fn write_out(trace: &Traced, op: &IoOp, path: Option<&str>) -> bool {
    let IoOp::Write { offset, buf, .. } = op else {
        return false;
    };
    if !is_data(path) || offset % PAGE as u64 != 0 || buf.len() % PAGE != 0 {
        return false;
    }
    let mut t = trace.lock().unwrap();
    let (during, number) = (t.during, t.ops);
    t.ops += 1;
    for (i, page) in buf.chunks(PAGE).enumerate() {
        t.writes.push(WriteOut {
            op: number,
            pid: Pid(offset / PAGE as u64 + i as u64),
            kind: storage::kind(page),
            owner: storage::owner(page),
            lsn: storage::lsn(page).0,
            during,
        });
    }
    true
}

pub fn options() -> OpenOptions {
    let mut create = CreateOptions::default();
    create.segment_size = Some(SEGMENT);
    OpenOptions {
        create: Some(create),
        workers: Some(2),
        pool_bytes: Some(48 * PAGE as u64),
        no_background_vacuum: true,
        ..OpenOptions::default()
    }
}

/// Returns the log of the migrations up to `version` past the first.
pub fn log(version: u8) -> Log {
    Log::new([APP, TAGS, UNTAG].into_iter().take(1 + version as usize))
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

/// The pages each relation's heap, directory, visibility map and key index held after an
/// operation, by the catalog's names, and every other allocated page, by `None`; free pages are
/// absent.
pub type Holdings = BTreeMap<Pid, Option<Rel>>;

/// The owner each user relation's pages carry: its heap's and its history heap's, which their
/// directory and visibility map pages carry too, its key index's and history index's, and its
/// large-value store's; and the marks a page of each takes.
#[derive(Clone, Debug, Default)]
pub struct Ids {
    pub heaps: BTreeMap<u32, Rel>,
    pub indexes: BTreeMap<u32, Rel>,
    pub large: BTreeSet<u32>,
    /// The owner of each relation's key index, which a read by key goes through.
    pub key_indexes: BTreeMap<Rel, u32>,
    /// The mark of each relation's heap and of its key index.
    pub marks: BTreeMap<Rel, (Quarantine, Quarantine)>,
}

impl Ids {
    /// Returns the user relation a page of `kind` carrying `owner` belongs to, `None` for a page
    /// no user relation holds.
    pub fn owner(&self, kind: Kind, owner: u32) -> Option<Rel> {
        match kind {
            Kind::Heap | Kind::Directory | Kind::VisibilityMap => self.heaps.get(&owner).copied(),
            Kind::Index => self.indexes.get(&owner).copied(),
            _ => None,
        }
    }

    /// Returns the relation whose heap or index mark `m` is, `None` for the database's.
    pub fn marked(&self, m: Quarantine) -> Option<Rel> {
        self.marks
            .iter()
            .find(|(_, (h, i))| *h == m || *i == m)
            .map(|(r, _)| *r)
    }
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

/// Returns whether the page of marks `db` writes holds the whole database's mark.
fn durably_whole<R: Runtime>(db: &Database<R>) -> bool {
    const DATABASE: u32 = 1;
    let pager = db.write_path().pager();
    if !pager.is_quarantined(Quarantine::Database) {
        return false;
    }
    let Ok(page) = pager.read(pager.quarantine_page()) else {
        return false;
    };
    if storage::kind(&page) != PageKind::Quarantine as u16 {
        return false;
    }
    let at = storage::HEADER_LEN;
    let word = |i: usize| u64::from_le_bytes(page[i..i + 8].try_into().unwrap());
    (0..word(at) as usize).any(|n| word(at + 8 + n * 8) as u32 == DATABASE)
}

/// Returns whether `db` holds a mark on the whole database, under which an open applies no
/// migration.
fn marked_whole<R: Runtime>(db: &Database<R>) -> bool {
    db.write_path()
        .pager()
        .quarantined()
        .contains(&Quarantine::Database)
}

/// Returns whether `rel` exists at schema `version`.
pub fn present(rel: Rel, version: u8) -> bool {
    rel != Rel::Tags || version == 1
}

fn holdings<R: Runtime>(db: &Database<R>, version: u8) -> Holdings {
    let w = db.write_path();
    let mut out = Holdings::new();
    for rel in RELS.into_iter().filter(|r| present(*r, version)) {
        let q = rel.qualified();
        let heap = w.heap_named(&q, rel.ctor()).expect("the relation's heap");
        let index = w
            .index_named(&q, "key id")
            .expect("the relation's key index");
        let mut pages = heap.owned_pages(w.pager()).expect("the heap's pages");
        pages.extend(index.tree().pages(w.pager()).expect("the index's pages"));
        out.extend(pages.into_iter().map(|p| (p, Some(rel))));
        if let Some(h) = w.history_heap_named(&q, rel.ctor()) {
            let pages = h.owned_pages(w.pager()).expect("the history heap's pages");
            out.extend(pages.into_iter().map(|p| (p, Some(rel))));
        }
        if let Some(ix) = w.history_index_named(&q) {
            let pages = ix
                .tree()
                .pages(w.pager())
                .expect("the history index's pages");
            out.extend(pages.into_iter().map(|p| (p, Some(rel))));
        }
        let large = heap
            .large()
            .owned_pages(w.pager())
            .expect("the store's pages");
        out.extend(large.into_iter().map(|p| (p, None)));
    }
    for p in w.pager().allocated().expect("the allocated pages") {
        out.entry(p).or_insert(None);
    }
    out
}

/// Add the owners of the relations of `db` at schema `version` to `out`.
fn ids<R: Runtime>(db: &Database<R>, version: u8, out: &mut Ids) {
    let w = db.write_path();
    for rel in RELS.into_iter().filter(|r| present(*r, version)) {
        let q = rel.qualified();
        let heap = w.heap_named(&q, rel.ctor()).expect("the relation's heap");
        let index = w
            .index_named(&q, "key id")
            .expect("the relation's key index");
        out.heaps.insert(heap.id().0, rel);
        out.indexes.insert(index.tree().owner(), rel);
        out.large.insert(heap.large().id());
        out.key_indexes.insert(rel, index.tree().owner());
        out.marks.insert(
            rel,
            (
                Quarantine::Heap(heap.id().0),
                Quarantine::Index(index.tree().owner()),
            ),
        );
        if let Some(h) = w.history_heap_named(&q, rel.ctor()) {
            out.heaps.insert(h.id().0, rel);
        }
        if let Some(ix) = w.history_index_named(&q) {
            out.indexes.insert(ix.tree().owner(), rel);
        }
    }
}

/// What a run of a sequence observed: what it acknowledged, the write-outs it numbered and the
/// relation holding each page after each operation, and how the run ended.
#[derive(Debug)]
pub struct Ran {
    pub acked: Acked,
    pub trace: Trace,
    pub held: Vec<Holdings>,
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
    /// The marks the file held when the migration that drops a relation began.
    pub marked_before_drop: BTreeSet<Quarantine>,
    pub files: Files,
}

/// The faults a run applies, set on the hook of its first process.
pub type Arm<'a> = &'a dyn Fn(&Hooked, &Traced);

/// The state one run carries across its processes.
struct Runner<'a> {
    ops: &'a [Op],
    trace: Traced,
    acked: Acked,
    held: Vec<Holdings>,
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
    marked_before_drop: BTreeSet<Quarantine>,
}

impl Runner<'_> {
    /// Open the file, and once more when the first open fails, as an operator would after a
    /// failure that may have cleared.
    fn open<R: Runtime>(&mut self, rt: R, version: u8) -> Option<Database<R>> {
        let first = Database::open_in(rt.clone(), FILE, &log(version), &options());
        match first.or_else(|_| Database::open_in(rt, FILE, &log(version), &options())) {
            Ok(db) => Some(db),
            Err(e) => {
                self.failed
                    .insert(self.next.saturating_sub(1), format!("open: {e:?}"));
                None
            }
        }
    }

    fn commit<R: Runtime>(&mut self, db: &Database<R>, op: &Op, i: usize) -> bool {
        self.pending = Some(op.clone());
        let done = write(db, op, self.acked.last());
        self.pending = None;
        match done {
            Ok(()) => {
                let rows = applied(self.acked.last().clone(), op);
                self.acked.states.push(rows);
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
    fn process(&mut self, hook: &Hooked) -> bool {
        let Some(mut db) = self.open(hook.clone(), self.acked.version) else {
            return false;
        };
        while self.next < self.ops.len() {
            let i = self.next;
            let op = &self.ops[i];
            self.trace.lock().unwrap().during = i;
            self.next += 1;
            match op {
                Op::Insert(rel, _) | Op::Update(rel, _) | Op::Delete(rel, _) if self.concurrent => {
                    self.beside_vacuum(hook, &db, op, *rel, i);
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
                    let pager = db.write_path().pager();
                    pager.flush_log();
                    pager.flush_pages(usize::MAX);
                    pager.checkpoint();
                }
                Op::Recycle => self.recycle(&db, i),
                Op::Damage => self.damage(&db, i),
                Op::Reopen | Op::Migrate => {
                    let durable = durably_whole(&db);
                    if *op == Op::Migrate && self.acked.version + 1 == LAST {
                        self.marked_before_drop = db.write_path().pager().quarantined();
                    }
                    if let Err(e) = db.close() {
                        self.failed.insert(i, format!("close: {e:?}"));
                    }
                    let version = if *op == Op::Migrate {
                        (self.acked.version + 1).min(LAST)
                    } else {
                        self.acked.version
                    };
                    let Some(again) = self.open(hook.clone(), version) else {
                        return false;
                    };
                    db = again;
                    let marked = marked_whole(&db);
                    let migrated = version != self.acked.version && applies(&db, version);
                    if version != self.acked.version && marked && !migrated {
                        self.under_mark.insert(i);
                    }
                    if durable && migrated {
                        self.applied_under_mark.insert(i);
                    }
                    if version != self.acked.version && (migrated || !marked) {
                        self.acked.version = version;
                        let rows = applied(self.acked.last().clone(), op);
                        self.acked.states.push(rows);
                    }
                }
                Op::Crash => {}
            }
            if self.observe {
                self.held.push(holdings(&db, self.acked.version));
                ids(&db, self.acked.version, &mut self.ids);
            }
            if *op == Op::Crash {
                return true;
            }
        }
        self.trace.lock().unwrap().during = self.ops.len();
        if let Err(e) = db.close() {
            self.failed.insert(self.ops.len(), format!("close: {e:?}"));
        }
        true
    }

    /// Commit `op` while a second thread vacuums `rel`, and count the commits the vacuum ran
    /// during.
    fn beside_vacuum(&mut self, hook: &Hooked, db: &Database<Hooked>, op: &Op, rel: Rel, i: usize) {
        let state = Arc::new(AtomicU8::new(0));
        let failure = Arc::new(Mutex::new(None));
        let (db2, state2, failure2) = (db.clone(), state.clone(), failure.clone());
        let thread = hook.spawn("vacuum", move || {
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
        hook.join(thread);
        if before < 2 && after >= 1 {
            self.overlaps += 1;
        }
        if let Some(e) = failure.lock().unwrap().take() {
            self.failed.insert(i, e);
        }
    }

    fn damage<R: Runtime>(&mut self, db: &Database<R>, i: usize) {
        let note = value(Rel::Scraps, i as i64, 0);
        let text = format!(r#"insert scraps {{ id: scraps.key({i}), note: "{note}" }}"#);
        if let Err(e) = db.query_text(&text, &[], &adtb::CallOptions::default()) {
            self.failed.insert(i, format!("{e:?}"));
            return;
        }
        let w = db.write_path();
        let heap = w.heap_named(&Rel::Scraps.qualified(), Rel::Scraps.ctor());
        match heap.expect("the heap of scraps").pages(w.pager()) {
            Ok(pages) => {
                self.damaged = Some(pages[0]);
                db.damage_page(pages[0], Damage::Checksum);
            }
            Err(e) => {
                self.failed.insert(i, format!("{e:?}"));
            }
        }
    }

    /// Change rows of `users` until the log has grown by three segments, then recycle it.
    fn recycle<R: Runtime>(&mut self, db: &Database<R>, i: usize) {
        let pager = db.write_path().pager();
        let start = pager.appended().0;
        while pager.appended().0 < start + 3 * SEGMENT {
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
        pager.recycle_log();
    }
}

/// Run `ops` from a new file on a simulation seeded `seed`, with `arm` setting the faults on the
/// hook every process shares, observing the holdings after each operation when `observe`, and
/// vacuuming a write's relation on a second thread during the write when `concurrent`.
pub fn run(seed: u64, ops: &[Op], arm: Arm<'_>, observe: bool, concurrent: bool) -> Ran {
    let mut sim = Sim::new(seed, Config::default());
    let mut runner = Runner {
        ops,
        trace: Traced::default(),
        acked: Acked {
            states: vec![Rows::new()],
            version: 0,
        },
        held: Vec::new(),
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
        marked_before_drop: BTreeSet::new(),
    };
    runner.trace.lock().unwrap().during = CREATING;
    let mut shared: Option<Hooked> = None;
    let mut cut = None;
    crash::silence_fatal_panics();
    loop {
        let crashing = ops[runner.next..].contains(&Op::Crash);
        if crashing {
            sim.crash_at(sim.now() + Duration::from_secs(500));
        }
        let mut whole = false;
        let outcome = crash::run_process(&mut sim, |rt| {
            let hook = match &shared {
                Some(h) => h.rebound(&rt),
                None => {
                    let h = Hooked::new(&rt);
                    arm(&h, &runner.trace);
                    shared = Some(h.clone());
                    h
                }
            };
            whole = runner.process(&hook);
            if whole && runner.next > 0 && ops[runner.next - 1] == Op::Crash {
                let mut io = rt.io();
                common::sleep(&rt, &mut io, Duration::from_secs(1000));
                unreachable!("the crash comes first");
            }
        });
        let planned = whole && runner.next > 0 && ops[runner.next - 1] == Op::Crash;
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
    let trace = runner.trace.lock().unwrap().clone();
    Ran {
        acked: runner.acked,
        trace,
        held: runner.held,
        ids: runner.ids,
        failed: runner.failed,
        cut,
        pending: runner.pending,
        overlaps: runner.overlaps,
        damaged: runner.damaged,
        under_mark: runner.under_mark,
        applied_under_mark: runner.applied_under_mark,
        marked_before_drop: runner.marked_before_drop,
        files: sim.files(),
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

pub const KINDS: [Kind; 11] = [
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
