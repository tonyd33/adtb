//! Every consumer of a file after a case: the open that settles, the catalog, a read of each
//! relation, verify, a scrub of the page, a backup with its proof and restore, and a dump with
//! its restore.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

use adtb::runtime::{IoError, OpKind, Runtime};
use adtb::testing::{Damage, Pid, Quarantine};
use adtb::{
    ArchiveTarget, CallOptions, Database, Error, FatalReason, OpenOptions, Proof, RestorePoint,
    prove_in, restore_in,
};
use adtb_harness::process;
use adtb_harness::search::Files;
use adtb_sim::{Action, Config, Match, Plan, Sim, SimRuntime};
use adtb_simkit::api;

use super::world::{self, FILE, Ran, Rel};

/// The consumers a case runs, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Consumer {
    Settle,
    Catalog,
    Read,
    Verify,
    Scrub,
    Backup,
    Dump,
}

pub const CONSUMERS: [Consumer; 7] = [
    Consumer::Settle,
    Consumer::Catalog,
    Consumer::Read,
    Consumer::Verify,
    Consumer::Scrub,
    Consumer::Backup,
    Consumer::Dump,
];

/// What a read of one relation answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// The version of each row, by its id.
    Rows(BTreeMap<i64, u32>),
    /// A value that is no value the sequence wrote.
    Garbage(String),
    /// Refused as quarantined, naming what.
    Refused(String),
    /// Any other failure.
    Failed(String),
}

/// One row of a `verify`: its constructor and where it found the problem.
pub type Finding = (String, String);

/// What a copy, restored from a backup or a dump, holds.
#[derive(Debug)]
pub struct Copy {
    pub reads: BTreeMap<Rel, Read>,
    pub verify: Result<Vec<Finding>, String>,
}

#[derive(Debug)]
pub struct Backup {
    pub made: Result<(), String>,
    pub proof: Option<Result<Proof, String>>,
    pub copy: Option<Result<Copy, String>>,
}

#[derive(Debug)]
pub struct Dumped {
    pub made: Result<(), String>,
    pub copy: Option<Result<Copy, String>>,
}

/// The reads by key of one relation: the keys, what they answered, and whether the plan probes
/// the key index.
#[derive(Debug)]
pub struct Lookups {
    pub keys: Vec<i64>,
    pub read: Read,
    pub probes: bool,
}

/// What a second open, with every page readable, served after the consumers closed the file.
#[derive(Debug)]
pub struct Reopened {
    pub open: Result<(), String>,
    pub marks: BTreeSet<Quarantine>,
    pub reads: BTreeMap<Rel, Read>,
}

/// What the consumers observed. A consumer that never ran left its part `None`.
#[derive(Debug, Default)]
pub struct Observed {
    pub open: Option<Result<(), String>>,
    /// Whether a read of the unreadable page failed while the first open ran.
    pub met_in_open: bool,
    /// The pages the damage records named once the first open returned, which that open mapped.
    pub recorded_at_open: BTreeSet<Pid>,
    /// The relations of module `app` the schema the open read holds.
    pub relations: Option<BTreeSet<String>>,
    pub reads: BTreeMap<Rel, Read>,
    pub lookups: BTreeMap<Rel, Lookups>,
    pub reopened: Option<Reopened>,
    /// The commit log status pages the open recorded as settled from the log.
    pub settled: BTreeSet<Pid>,
    /// The page the scrub ran on.
    pub scrubbed: Option<Pid>,
    pub verify: Option<Result<Vec<Finding>, String>>,
    pub scrub: Option<Vec<String>>,
    pub marks: Option<BTreeSet<Quarantine>>,
    pub backup: Option<Backup>,
    pub dump: Option<Dumped>,
    /// A panic of the engine, by its message.
    pub panic: Option<String>,
    /// The files once every consumer ran, after the recovery their open ran.
    pub files: Files,
}

fn no_create() -> OpenOptions {
    OpenOptions {
        create: None,
        ..world::options()
    }
}

const BACKUP: &str = "bk";
const DUMP: &str = "dm";

/// Returns the rows of a read, or what refused it.
fn read<R: Runtime>(db: &Database<R>, rel: Rel) -> Read {
    match db.query_text(&rel.read(), &[], &CallOptions::default()) {
        Ok(mut rows) => {
            let printed = api::printed(rows.pop().expect("a read answers rows"));
            let mut out = BTreeMap::new();
            for row in printed {
                match parsed(rel, &row) {
                    Some((id, v)) if !out.contains_key(&id) => {
                        out.insert(id, v);
                    }
                    _ => return Read::Garbage(row.chars().take(80).collect()),
                }
            }
            Read::Rows(out)
        }
        Err(e) => refusal(&e),
    }
}

fn refusal(e: &Error) -> Read {
    match e {
        Error::Fatal {
            reason: FatalReason::Quarantined { what },
        } => Read::Refused(what.clone()),
        other => Read::Failed(format!("{other:?}")),
    }
}

/// Returns the id and version of a printed value of `rel`, `None` for one no write gave.
fn parsed(rel: Rel, row: &str) -> Option<(i64, u32)> {
    let text = row.strip_prefix('"')?.strip_suffix('"')?;
    let rest = text.strip_prefix(rel.name())?;
    let (id, rest) = rest.split_once('v')?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let (id, v): (i64, u32) = (id.parse().ok()?, digits.parse().ok()?);
    (world::value(rel, id, v) == text).then_some((id, v))
}

fn relations(version: u8) -> Vec<Rel> {
    [Rel::Users, Rel::Docs, Rel::Tags]
        .into_iter()
        .filter(|r| world::present(*r, version))
        .collect()
}

/// Returns the rows of a `verify` of the whole database on `db`.
fn verify<R: Runtime>(db: &Database<R>) -> Result<Vec<Finding>, String> {
    let rows = db.verify(None).map_err(|e| format!("{e:?}"))?;
    let row = |r: String| {
        let (ctor, rest) = r.split_once(" {").unwrap_or((&r, ""));
        let at = rest.split_once("where: ").map_or("", |(_, at)| at);
        (ctor.to_owned(), at.trim_end_matches(" }").to_owned())
    };
    Ok(api::sorted(rows).into_iter().map(row).collect())
}

/// Returns what a restore of `archive` to `path` holds, opened with the log to `version`.
fn restored<R: Runtime>(rt: &R, archive: &str, path: &str, version: u8) -> Result<Copy, String> {
    let target = ArchiveTarget::path(archive).expect("a target path");
    restore_in(
        rt.clone(),
        &target,
        path,
        RestorePoint::Latest,
        &no_create(),
    )
    .map_err(|e| format!("restore: {e:?}"))?;
    let db = Database::open_in(rt.clone(), path, &world::log(version), &no_create())
        .map_err(|e| format!("open: {e:?}"))?;
    let reads = relations(version)
        .into_iter()
        .map(|r| (r, read(&db, r)))
        .collect();
    let verify = verify(&db);
    db.close().map_err(|e| format!("close: {e:?}"))?;
    Ok(Copy { reads, verify })
}

/// Run every consumer on the file `ran` left, on a simulation seeded `seed`, with every read
/// of `pid` failing when `lasting`, the log holding the migrations up to `version`, and `keys`
/// read by key from each relation. Under `lasting`, open the file again after with `pid`
/// readable.
pub fn run(
    seed: u64,
    ran: &Ran,
    pid: Pid,
    lasting: bool,
    version: u8,
    keys: &BTreeMap<Rel, Vec<i64>>,
) -> Observed {
    let mut sim = Sim::with_files(seed, Config::default(), ran.files.clone());
    let mut seen = Observed::default();
    let unreadable = Match::kind(OpKind::Read)
        .path(FILE)
        .covers(world::DATA.range(pid).start as u64);
    let failed = sim.logged(unreadable.clone());
    let mut failed_in_open = 0;
    let ran = catch_unwind(AssertUnwindSafe(|| {
        process::run(&mut sim, |rt| {
            if lasting {
                rt.plan(Plan::new().rule(unreadable, Action::Fail(IoError::Failed)));
            }
            let mut opened = || failed_in_open = failed.len();
            consume(&rt, pid, version, keys, &mut opened, &mut seen);
        });
        if lasting {
            seen.reopened = Some(reopen(seed, sim.files(), version));
        }
    }));
    seen.met_in_open = lasting && failed_in_open > 0;
    if let Err(payload) = ran {
        let text = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_else(|| "a panic".to_owned());
        seen.panic = Some(text);
    }
    seen.files = sim.files();
    seen
}

/// Returns what an open of `files` with every page readable serves.
fn reopen(seed: u64, files: Files, version: u8) -> Reopened {
    let mut sim = Sim::with_files(seed, Config::default(), files);
    let mut out = Reopened {
        open: Ok(()),
        marks: BTreeSet::new(),
        reads: BTreeMap::new(),
    };
    process::run(&mut sim, |rt| {
        match Database::open_in(rt, FILE, &world::log(version), &no_create()) {
            Ok(db) => {
                out.marks = db.testing().probe().marks();
                out.reads = relations(version)
                    .into_iter()
                    .map(|r| (r, read(&db, r)))
                    .collect();
                if let Err(e) = db.close() {
                    out.open = Err(format!("close: {e:?}"));
                }
            }
            Err(e) => out.open = Err(format!("open: {e:?}")),
        }
    });
    out
}

/// Returns the rows of `rel` with keys `keys`, read by key, and whether the plan probes the key
/// index.
fn lookups<R: Runtime>(db: &Database<R>, rel: Rel, keys: &[i64]) -> Lookups {
    let one = |k: i64| rel.lookup(k);
    let probes = db
        .query_text(
            &format!("explain {}", one(keys[0])),
            &[],
            &CallOptions::default(),
        )
        .is_ok_and(|mut r| {
            r.pop()
                .is_some_and(|v| api::printed(v).iter().any(|t| t.contains("Probe ")))
        });
    let mut rows = BTreeMap::new();
    for &k in keys {
        match db.query_text(&one(k), &[], &CallOptions::default()) {
            Ok(mut answer) => {
                for row in api::printed(answer.pop().expect("a read answers rows")) {
                    match parsed(rel, &row) {
                        Some((id, v)) if id == k && !rows.contains_key(&id) => {
                            rows.insert(id, v);
                        }
                        _ => {
                            return Lookups {
                                keys: keys.to_vec(),
                                read: Read::Garbage(row.chars().take(80).collect()),
                                probes,
                            };
                        }
                    }
                }
            }
            Err(e) => {
                return Lookups {
                    keys: keys.to_vec(),
                    read: refusal(&e),
                    probes,
                };
            }
        }
    }
    Lookups {
        keys: keys.to_vec(),
        read: Read::Rows(rows),
        probes,
    }
}

/// Run the consumers on the source and on its copies, calling `opened` once the first open
/// returns.
fn consume(
    rt: &SimRuntime,
    pid: Pid,
    version: u8,
    keys: &BTreeMap<Rel, Vec<i64>>,
    opened: &mut dyn FnMut(),
    seen: &mut Observed,
) {
    let open = Database::open_in(rt.clone(), FILE, &world::log(version), &no_create());
    opened();
    let db = match open {
        Ok(db) => db,
        Err(e) => {
            seen.open = Some(Err(format!("{e:?}")));
            return;
        }
    };
    seen.open = Some(Ok(()));
    seen.recorded_at_open = db
        .testing()
        .probe()
        .records()
        .pages()
        .keys()
        .copied()
        .collect();
    let s = db.schema();
    let app = s
        .relations
        .iter()
        .filter(|r| s.module(r.module).name == "app");
    seen.relations = Some(app.map(|r| r.name.to_string()).collect());
    for rel in relations(version) {
        seen.reads.insert(rel, read(&db, rel));
        if let Some(ks) = keys.get(&rel).filter(|k| !k.is_empty()) {
            seen.lookups.insert(rel, lookups(&db, rel, ks));
        }
    }
    seen.verify = Some(verify(&db));
    seen.scrubbed = Some(pid);
    seen.scrub = Some(
        db.testing()
            .scrub_page(pid)
            .iter()
            .map(|f| format!("{f:?}"))
            .collect(),
    );
    seen.marks = Some(db.testing().probe().marks());
    seen.settled = db
        .testing()
        .probe()
        .records()
        .pages()
        .iter()
        .filter(|(_, d)| matches!(d, Damage::Settled { .. }))
        .map(|(p, _)| *p)
        .collect();
    let text = |t: &str| {
        db.query_text(t, &[], &CallOptions::default())
            .map(drop)
            .map_err(|e| format!("{e:?}"))
    };
    let made = text(&format!("backup to \"{BACKUP}\""));
    let (proof, copy) = if made.is_ok() {
        let target = ArchiveTarget::path(BACKUP).expect("a target path");
        let proof = prove_in(rt.clone(), &target, &world::log(version), &no_create());
        (
            Some(proof.map_err(|e| format!("{e:?}"))),
            Some(restored(rt, BACKUP, "copy.adt", version)),
        )
    } else {
        (None, None)
    };
    seen.backup = Some(Backup { made, proof, copy });
    let made = text(&format!("dump to \"{DUMP}\""));
    let copy = made
        .is_ok()
        .then(|| restored(rt, DUMP, "dumped.adt", version));
    seen.dump = Some(Dumped { made, copy });
    if let Err(e) = db.close() {
        seen.open = Some(Err(format!("close: {e:?}")));
    }
}
