use adtb::runtime::{IoError, OpKind};
use adtb::testing::{Knobs, PageKind, Pid, page};
use adtb::{
    CallOptions, Error, FatalReason, Log, OpenError, OpenOptions, Pins, Registration, TaskOptions,
    TxnOptions,
};
use adtb_sim::{Action, Config, Match, Outcome, Plan, Sim, SimRuntime, Submitted};
use adtb_simkit::{api, open};
use adtb_testkit::damage::DataFile;
use adtb_testkit::search::Files;
use adtb_testkit::{crash, process};

const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
";

const MIGRATION: &str = "module app
add type Note = { id: Key notes, text: Text }
add relation notes : Note key id generate sequence
";

const FILE: &str = "app.adt";
const LOG_DIR: &str = "app.adt-log";
const DATA: DataFile = DataFile::new(FILE, 8192);
const NAMES: &str = "from u in users select u.name";
const LATE: &str = "this open read pages it could not check, so the database must be reopened";

fn options() -> OpenOptions {
    OpenOptions {
        testing: Knobs::default().with_segment_size(8192),
        registrations: vec![Registration::new(
            "reads",
            "import app\nquery names () -> Set Text = from u in users select u.name\n",
        )],
        ..open::options(2)
    }
}

/// Returns the names the database holds, sorted.
fn names() -> Vec<String> {
    let mut names: Vec<String> = (0..400).map(|i| format!("\"u{i}\"")).collect();
    names.sort();
    names
}

/// Returns the files of a database holding [`names`], written over several commits, its log
/// recycled into the page LSN map, and closed.
fn made() -> Files {
    let mut sim = Sim::new(7, Config::default());
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        for chunk in names().chunks(8) {
            let rows: Vec<String> = chunk
                .iter()
                .map(|n| format!("insert users {{ name: {n} }}"))
                .collect();
            api::results(&db, &rows.join("\n"));
        }
        db.testing().flush();
        db.testing().checkpoint();
        assert!(
            db.testing().recycle_log() > 0,
            "the recycle removes a segment"
        );
        db.close().expect("the database closes");
    });
    sim.files()
}

fn storage(detail: &str) -> Error {
    Error::Fatal {
        reason: FatalReason::Storage {
            detail: detail.to_owned(),
        },
    }
}

/// Returns each page of the data file in `files` of kind `kind`.
fn pages_of(files: &Files, kind: PageKind) -> Vec<Pid> {
    let pages = DATA.bytes(files).len() / 8192;
    (0..pages as u64)
        .map(Pid)
        .filter(|&p| page::kind(DATA.page(files, p)) == kind as u16)
        .collect()
}

/// What a case fails while the open arms its read-in gate.
#[derive(Clone, Debug)]
enum Target {
    /// Every read of this page.
    Page(Pid),
    /// The op of the log numbered so among those `ops` matches in the open, and every op like it
    /// after the open: every listing, or every read of the same segment at the same offset.
    Log { ops: LogOps, n: u64, op: Submitted },
}

#[derive(Clone, Copy, Debug)]
enum LogOps {
    Listings,
    SegmentReads,
}

impl LogOps {
    fn matched(self) -> Match {
        match self {
            LogOps::Listings => Match::kind(OpKind::List).path(LOG_DIR),
            LogOps::SegmentReads => Match::kind(OpKind::Read).prefixed(&format!("{LOG_DIR}/")),
        }
    }
}

/// Returns each op `ops` matches in an open of `files` that meets no fault.
fn ops_of(files: &Files, ops: LogOps) -> Vec<Submitted> {
    let mut sim = Sim::with_files(7, Config::default(), files.clone());
    process::run(&mut sim, |rt| {
        let log = rt.logged(ops.matched());
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        let entries = log.entries();
        db.close().expect("the database closes");
        entries
    })
}

/// Returns the plan failing `op` of the open, the plan failing every op like it after the open,
/// and the detail of the storage failure a call names.
fn failing(op: &Submitted, ops: LogOps, n: u64) -> (Plan, Plan, String) {
    let path = op.path().expect("a log op names its path");
    match ops {
        LogOps::Listings => (
            Plan::new().at(n, fail()),
            Plan::new().rule(ops.matched(), fail()),
            format!("the log at `{LOG_DIR}` cannot be read: the device refused the operation"),
        ),
        LogOps::SegmentReads => {
            let offset = op.offset().expect("a read has an offset");
            let name = path.rsplit_once('/').map_or(path, |(_, name)| name);
            let base = u64::from_str_radix(name, 16).expect("a segment's name is its base LSN");
            (
                Plan::new().at(n, fail()),
                Plan::new().rule(Match::kind(OpKind::Read).path(path).covers(offset), fail()),
                format!("the log block holding LSN {} cannot be read", base + offset),
            )
        }
    }
}

fn fail() -> Action {
    Action::Fail(IoError::Failed)
}

fn reads_of(pid: Pid) -> Match {
    Match::kind(OpKind::Read)
        .path(FILE)
        .covers(DATA.range(pid).start as u64)
}

/// Run `target` over the files `files`: the open that meets it, the call that meets it still, the
/// call after it clears, `verify` and `dump` in both, and the open after. Returns how the open
/// ran, `None` when the target failed the open itself.
fn case(files: &Files, target: &Target) -> Option<Met> {
    let mut sim = Sim::with_files(7, Config::default(), files.clone());
    let mut open_done = false;
    let mut unarmed = None;
    let body = |rt: SimRuntime| unarmed = met(&rt, target, &mut open_done);
    let outcome = crash::run_process(&mut sim, Box::new(body));
    if outcome == Outcome::Crashed && !open_done {
        return None;
    }
    assert_eq!(
        Outcome::Finished,
        outcome,
        "{target:?}: the opened process finishes"
    );
    let unarmed = unarmed?;
    if let Met::Unarmed { .. } = unarmed {
        let session = sim.files();
        assert_eq!(
            Vec::<String>::new(),
            removed(files, &session),
            "{target:?}: the unarmed session removes no segment of the log"
        );
        assert_eq!(
            Vec::<Pid>::new(),
            changed(&session_baseline(files, target), &session),
            "{target:?}: the unarmed session's calls write no page but the header's"
        );
    }
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        assert_eq!(
            names(),
            api::query(&db, NAMES),
            "{target:?}: the next open serves"
        );
        db.close().expect("the database closes");
    });
    Some(unarmed)
}

/// Returns the files an open of `files` meeting `target` leaves when it writes out the pages its
/// recovery changed and closes.
fn session_baseline(files: &Files, target: &Target) -> Files {
    let mut sim = Sim::with_files(7, Config::default(), files.clone());
    let body = |rt: SimRuntime| {
        rt.plan(plans(&rt, target).0);
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
        rt.disarm();
        rt.plan(Plan::new());
        db.testing().flush();
        db.close().expect("the database closes");
    };
    let outcome = crash::run_process(&mut sim, Box::new(body));
    assert_eq!(
        Outcome::Finished,
        outcome,
        "{target:?}: the open alone finishes"
    );
    sim.files()
}

/// Returns each page of the data file but the header's two that differs between `before` and
/// `after` or is new in `after`.
fn changed(before: &Files, after: &Files) -> Vec<Pid> {
    let pages = |files: &Files| DATA.bytes(files).len() / 8192;
    (2..pages(after) as u64)
        .map(Pid)
        .filter(|&p| p.0 as usize >= pages(before) || DATA.page(before, p) != DATA.page(after, p))
        .collect()
}

/// Returns each segment of the log in `before` that `after` lacks.
fn removed(before: &Files, after: &Files) -> Vec<String> {
    let prefix = format!("{LOG_DIR}/");
    before
        .keys()
        .filter(|name| name.starts_with(&prefix) && !after.contains_key(*name))
        .cloned()
        .collect()
}

/// Returns the plan an open meeting `target` runs under, the plan of the calls while the failure
/// lasts, and the detail of the storage failure a call names. Arm the log of `rt` for a log
/// target.
fn plans(rt: &SimRuntime, target: &Target) -> (Plan, Plan, String) {
    match target {
        Target::Page(pid) => (
            Plan::new().rule(reads_of(*pid), fail()),
            Plan::new().rule(reads_of(*pid), fail()),
            format!("page {} is unreadable", pid.0),
        ),
        Target::Log { ops, n, op } => {
            rt.arm(ops.matched());
            failing(op, *ops, *n)
        }
    }
}

/// Open the database meeting `target`, setting `opened` once it opens, then run the calls
/// [`case`] states. Returns how the open ran, `None` when the open failed.
fn met(rt: &SimRuntime, target: &Target, opened: &mut bool) -> Option<Met> {
    let (plan, lasting, detail) = plans(rt, target);
    rt.plan(plan);
    let db = open::open(rt, FILE, &Log::new([APP]), &options()).ok()?;
    *opened = true;
    rt.disarm();
    rt.plan(lasting);
    let first = db.query_text(NAMES, &[], &CallOptions::default()).err();
    if first.is_none() {
        rt.plan(Plan::new());
        db.close().expect("the database closes");
        return Some(Met::Armed);
    }
    let label = format!("{target:?}");
    let named = first != Some(storage(LATE))
        || !matches!(
            target,
            Target::Log {
                ops: LogOps::SegmentReads,
                ..
            }
        );
    if named {
        assert_eq!(
            Some(storage(&detail)),
            first,
            "{label}: the call names the failure"
        );
        api::results(&db, "verify");
        let dumped = db
            .query_text("dump to \"d1\"", &[], &CallOptions::default())
            .err();
        let unlogged = matches!(target, Target::Log { .. }).then(|| storage(&detail));
        assert_eq!(
            unlogged, dumped,
            "{label}: a dump answers unless it cannot read the log"
        );
        rt.plan(Plan::new());
        assert_eq!(
            storage(LATE),
            api::failure(&db, NAMES),
            "{label}: once the read succeeds, a call asks for a reopen"
        );
    }
    rt.plan(Plan::new());
    assert_eq!(
        storage(LATE),
        api::failure(&db, NAMES),
        "{label}: every later call asks for a reopen"
    );
    api::results(&db, "verify");
    api::results(&db, "dump to \"d2\"");
    db.close().expect("the database closes");
    Some(Met::Unarmed { named })
}

/// How an open meeting a target ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Met {
    /// The open armed its gate.
    Armed,
    /// The open was left unarmed. Unless `named`, the first call's retry read none of what failed
    /// and asked for a reopen at once.
    Unarmed { named: bool },
}

#[test]
fn a_read_failing_while_the_open_arms_fails_each_call_as_storage_until_a_reopen() {
    crash::silence_fatal_panics();
    let files = made();
    let mut reached = Vec::new();
    for kind in [PageKind::LsnMap, PageKind::DamageList] {
        let pids = pages_of(&files, kind);
        let unarmed = pids
            .iter()
            .filter(|&&p| case(&files, &Target::Page(p)) == Some(Met::Unarmed { named: true }))
            .count();
        reached.push((format!("{kind:?}"), pids.len(), unarmed));
    }
    for ops in [LogOps::Listings, LogOps::SegmentReads] {
        let seen = ops_of(&files, ops);
        let unarmed = (0..)
            .zip(&seen)
            .filter(|&(n, op)| {
                let target = Target::Log {
                    ops,
                    n,
                    op: op.clone(),
                };
                case(&files, &target) == Some(Met::Unarmed { named: true })
            })
            .count();
        reached.push((format!("{ops:?}"), seen.len(), unarmed));
    }
    for (what, of, unarmed) in &reached {
        assert!(
            *unarmed >= 1,
            "{what}: no case of {of} left the open unarmed with a call naming the failure: {reached:?}"
        );
    }
}

/// An entry a client calls first on an open left unarmed.
#[derive(Clone, Copy, Debug)]
enum Entry {
    Query,
    Begin,
    Backup,
    Registered,
    Task,
}

const ENTRIES: [Entry; 5] = [
    Entry::Query,
    Entry::Begin,
    Entry::Backup,
    Entry::Registered,
    Entry::Task,
];

/// Returns what `entry` answers on `db`, its rows dropped.
fn entered(db: &adtb::Database<SimRuntime>, entry: Entry) -> Result<(), Error> {
    let opts = CallOptions::default();
    match entry {
        Entry::Query => db.query_text(NAMES, &[], &opts).map(drop),
        Entry::Begin => db.begin(&TxnOptions::default()).map(drop),
        Entry::Backup => db.query_text("backup to \"b\"", &[], &opts).map(drop),
        Entry::Registered => db.registered("reads")?.call(1, &[], &opts).map(drop),
        Entry::Task => {
            let authority = db.role_authority();
            let owner = authority.session("owner")?;
            let bound = authority.bind(&owner, "reads", &Pins::default())?;
            bound
                .submit(1, Vec::new(), TaskOptions::default())
                .map(drop)
        }
    }
}

#[test]
fn every_entry_retries_the_failed_read_and_an_open_given_migrations_fails_naming_it() {
    crash::silence_fatal_panics();
    let files = made();
    let list = pages_of(&files, PageKind::DamageList)[0];
    let failing = || Plan::new().rule(reads_of(list), fail());
    for entry in ENTRIES {
        let mut sim = Sim::with_files(7, Config::default(), files.clone());
        let got = process::run(&mut sim, |rt| {
            rt.plan(failing());
            let db = open::opened(&rt, FILE, &Log::new([APP]), &options());
            rt.plan(Plan::new());
            let got = entered(&db, entry);
            db.close().expect("the database closes");
            got
        });
        assert_eq!(
            Err(storage(LATE)),
            got,
            "{entry:?}: the first call after the read clears tries it again"
        );
    }
    let mut sim = Sim::with_files(7, Config::default(), files);
    let got = process::run(&mut sim, |rt| {
        rt.plan(failing());
        let opened = adtb::Database::open_in(rt, FILE, &Log::new([APP, MIGRATION]), &options());
        opened.map(drop).map_err(|e| match e {
            OpenError::Io(e) => (e.kind(), e.to_string()),
            e => panic!("the open fails as I/O: {e:?}"),
        })
    });
    assert_eq!(
        Err((
            std::io::ErrorKind::Other,
            format!("page {} is unreadable", list.0)
        )),
        got,
        "an open given migrations fails naming the read"
    );
}
