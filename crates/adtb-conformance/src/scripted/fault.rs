//! Faults a script plants on the device and the engine, restores of a dump or backup, and probes
//! of the marks, the damage records and the device's files.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use adtb::runtime::{AlignedBuf, Done, Io, Op, OpKind, Runtime, Threads};
use adtb::testing::{Damage, LogFinding, LogProblem, Object, Pid, Plant, Quarantine, Testing};
use adtb::{ArchiveTarget, Database, OpenOptions, Position, RestorePoint, restore_report_in};
use adtb_sim::{Action, Match, OpLog, Plan, Planned, SimRuntime};

use super::session::{Differs, refused};
use super::{Head, Step, pages};

/// The byte of a page a flip inverts, inside its body.
const FLIPPED: usize = 300;

/// A page named by the structure that holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Page {
    /// The first data page of the heap of a constructor.
    Heap { relation: String, ctor: String },
    /// The root of the directory of the heap of a constructor.
    Directory { relation: String, ctor: String },
    /// The first page of the store of the large values of the heap of a constructor.
    Large { relation: String, ctor: String },
    /// The root of an index, by its clause as the schema writes it.
    Index { relation: String, clause: String },
    /// The root page of the catalog.
    Catalog,
    /// The root page of the commit log.
    CommitLog,
    /// The status page of the commit log holding the newest transaction ids.
    Status,
}

impl Page {
    pub(super) fn parse(text: &str) -> Option<Page> {
        Some(match text.split_once(' ').unwrap_or((text, "")) {
            ("heap", rest) => {
                let (relation, ctor) = heap_words(rest)?;
                Page::Heap { relation, ctor }
            }
            ("directory", rest) => {
                let (relation, ctor) = heap_words(rest)?;
                Page::Directory { relation, ctor }
            }
            ("large", rest) => {
                let (relation, ctor) = heap_words(rest)?;
                Page::Large { relation, ctor }
            }
            ("index", rest) => {
                let (relation, clause) = rest.split_once(' ')?;
                Page::Index {
                    relation: relation.to_owned(),
                    clause: clause.to_owned(),
                }
            }
            ("catalog", "") => Page::Catalog,
            ("commit", "log") => Page::CommitLog,
            ("status", "") => Page::Status,
            _ => return None,
        })
    }

    pub(super) fn pid(&self, t: Testing<'_, SimRuntime>) -> Result<Pid, String> {
        self.found(t)
            .map_err(|e| format!("the page `{self}` names is not found: {e}"))
    }

    fn found(&self, t: Testing<'_, SimRuntime>) -> Result<Pid, String> {
        let first = |pages: Vec<Pid>| pages.first().copied().ok_or("it holds no page");
        match self {
            Page::Heap { relation, ctor } => {
                let heap = t.heap(relation, ctor).ok_or("no such constructor")?;
                t.pages(heap)
                    .map_err(|e| format!("{e:?}"))
                    .and_then(|p| Ok(first(p)?))
            }
            Page::Directory { relation, ctor } => {
                let heap = t.heap(relation, ctor).ok_or("no such constructor")?;
                t.root(heap).map_err(|e| format!("{e:?}"))
            }
            Page::Large { relation, ctor } => {
                let large = t.large(relation, ctor).ok_or("no such constructor")?;
                t.pages(large)
                    .map_err(|e| format!("{e:?}"))
                    .and_then(|p| Ok(first(p)?))
            }
            Page::Index { relation, clause } => {
                let index = t.index(relation, clause).ok_or("no such index")?;
                t.root(index).map_err(|e| format!("{e:?}"))
            }
            Page::Catalog => t.root(Object::Catalog).map_err(|e| format!("{e:?}")),
            Page::CommitLog => Ok(t.commit_log().root()),
            Page::Status => {
                let clog = t.commit_log();
                let last = clog.status().values().next_back().copied();
                last.ok_or_else(|| "the commit log has no status page".to_owned())
            }
        }
    }
}

/// Returns the relation and constructor `rest` names, two words.
fn heap_words(rest: &str) -> Option<(String, String)> {
    let (relation, ctor) = rest.split_once(' ')?;
    (!ctor.contains(' ')).then(|| (relation.to_owned(), ctor.to_owned()))
}

impl fmt::Display for Page {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Page::Heap { relation, ctor } => write!(f, "heap {relation} {ctor}"),
            Page::Directory { relation, ctor } => write!(f, "directory {relation} {ctor}"),
            Page::Large { relation, ctor } => write!(f, "large {relation} {ctor}"),
            Page::Index { relation, clause } => write!(f, "index {relation} {clause}"),
            Page::Catalog => f.write_str("catalog"),
            Page::CommitLog => f.write_str("commit log"),
            Page::Status => f.write_str("status"),
        }
    }
}

/// What a fault does to its page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fault {
    /// Refuse every later read of the page as failing its checksum, recording the damage.
    Damage,
    /// Invert a byte of the page's body on the device once the process closes the file.
    Flip,
    /// Lose every later write of the process that covers the page.
    Lose,
    /// Tear the next write that covers the page.
    Tear,
    /// Edit the page as the step's body states, keeping its checksum valid.
    Scribble,
    /// Allocate a page of the page's kind and owner that no structure names.
    Leak,
    /// Free the page in the allocation bitmap, whatever still names it.
    Free,
}

/// An object a mark names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Marked {
    Database,
    Heap { relation: String, ctor: String },
    Index { relation: String, clause: String },
}

/// Where a process crashes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CrashAt {
    /// At the first op of the kind on the path, before it completes.
    Op(OpKind, String),
    /// At the device op after the next N, before it completes.
    After(u64),
}

/// What a probe prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Probed {
    Marks,
    Damage,
    /// The rows of the run of `verify` that holds the database.
    Holding,
    /// The directory positions and pages of the heap of a constructor, with each page's slots
    /// unless `pages` only.
    Heap {
        relation: String,
        ctor: String,
        pages: bool,
    },
    /// The device's files whose path begins with the text.
    Files(String),
    /// The structures the catalog names for the relations of the engine's modules when true, else
    /// of the others.
    Catalog(bool),
}

/// A restore of the dump or backup at `source` to the new file `to`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Restore {
    source: String,
    to: String,
    /// The point, `None` for the latest: `version N` or a position.
    at: Option<String>,
    /// The error the restore is refused with, in textual syntax.
    want: Option<String>,
}

impl Restore {
    /// Returns whether the restore states a refusal.
    pub(super) fn refused(&self) -> bool {
        self.want.is_some()
    }

    /// Returns the refusal the restore states, to place positions and instants in.
    pub(super) fn want_mut(&mut self) -> Option<&mut String> {
        self.want.as_mut()
    }

    /// Returns the point the restore states, to place positions in.
    pub(super) fn at_mut(&mut self) -> Option<&mut String> {
        self.at.as_mut()
    }
}

/// Returns the head of the fault, restore or probe directive `word` begins.
pub(super) fn head(word: &str, rest: &str) -> Option<Head> {
    let fault = |fault| {
        Some(Head::Fault {
            fault,
            page: Page::parse(rest)?,
        })
    };
    match word {
        "mark" => Some(Head::Mark(marked(rest)?)),
        "damage" => fault(Fault::Damage),
        "flip" => fault(Fault::Flip),
        "lose" => fault(Fault::Lose),
        "tear" => fault(Fault::Tear),
        "scribble" => fault(Fault::Scribble),
        "leak" => fault(Fault::Leak),
        "free" => fault(Fault::Free),
        "crash" => Some(Head::Crash(crash_at(rest)?)),
        "restore" => Some(Head::Restore(restore(rest)?)),
        "fatal" if !rest.is_empty() => Some(Head::Fatal(rest.to_owned())),
        "probe" => Some(Head::Probe(
            match rest.split_once(' ').unwrap_or((rest, "")) {
                ("marks", "") => Probed::Marks,
                ("damage", "") => Probed::Damage,
                ("holding", "verify") => Probed::Holding,
                ("heap", rest) => {
                    let (rest, pages) = match rest.strip_suffix(" pages") {
                        Some(rest) => (rest, true),
                        None => (rest, false),
                    };
                    let (relation, ctor) = heap_words(rest)?;
                    Probed::Heap {
                        relation,
                        ctor,
                        pages,
                    }
                }
                ("files", prefix) => Probed::Files(prefix.to_owned()),
                ("catalog", "") => Probed::Catalog(false),
                ("catalog", "engine") => Probed::Catalog(true),
                ("plan", "") => return Some(Head::ProbePlan),
                ("checks", name) if !name.is_empty() => {
                    return Some(Head::ProbeChecks(name.to_owned()));
                }
                _ => return None,
            },
        )),
        _ => None,
    }
}

fn marked(rest: &str) -> Option<Marked> {
    Some(match Page::parse(rest) {
        _ if rest == "database" => Marked::Database,
        Some(Page::Heap { relation, ctor }) => Marked::Heap { relation, ctor },
        Some(Page::Index { relation, clause }) => Marked::Index { relation, clause },
        _ => return None,
    })
}

fn crash_at(rest: &str) -> Option<CrashAt> {
    if let Some(n) = rest.strip_prefix("after ") {
        return Some(CrashAt::After(n.parse().ok()?));
    }
    let (kind, path) = rest.strip_prefix("at ")?.split_once(' ')?;
    let kind = match kind {
        "open" => OpKind::Open,
        "create_new" => OpKind::CreateNew,
        "write" => OpKind::Write,
        "allocate" => OpKind::Allocate,
        "sync" => OpKind::Sync,
        "sync_dir" => OpKind::SyncDir,
        "remove" => OpKind::Remove,
        "remove_dir" => OpKind::RemoveDir,
        "rename" => OpKind::Rename,
        "create_dir" => OpKind::CreateDir,
        _ => return None,
    };
    (!path.contains(' ')).then(|| CrashAt::Op(kind, path.to_owned()))
}

fn restore(rest: &str) -> Option<Restore> {
    let (rest, want) = match rest.split_once(" error ") {
        Some((rest, v)) => (rest, Some(format!("Rejected {{ reason: {v} }}"))),
        None => match rest.split_once(" fatal ") {
            Some((rest, v)) => (rest, Some(format!("Fatal {{ reason: {v} }}"))),
            None => (rest, None),
        },
    };
    let (rest, at) = match rest.split_once(" at ") {
        Some((rest, at)) => (rest, Some(at.to_owned())),
        None => (rest, None),
    };
    let (source, to) = rest.split_once(" to ")?;
    let word = |w: &str| !w.is_empty() && !w.contains(' ');
    (word(source) && word(to)).then(|| Restore {
        source: source.to_owned(),
        to: to.to_owned(),
        at,
        want,
    })
}

/// Returns the restore point `at` states: `version N`, or a position in textual syntax.
fn point(at: Option<&str>) -> Option<RestorePoint> {
    let Some(at) = at else {
        return Some(RestorePoint::Latest);
    };
    if let Some(v) = at.strip_prefix("version ") {
        return Some(RestorePoint::Version(v.parse().ok()?));
    }
    let fields = at.strip_prefix("{ epoch: ")?.strip_suffix(" }")?;
    let (epoch, lsn) = fields.split_once(", lsn: ")?;
    Some(RestorePoint::Position(Position {
        epoch: epoch.parse().ok()?,
        lsn: lsn.parse().ok()?,
    }))
}

/// The faults a script planted and what they still owe.
pub(super) struct State {
    /// The file each open opens, the last a restore wrote.
    file: String,
    /// The name of each page a fault named or allocated, by page, to print damage records and
    /// findings by.
    roles: BTreeMap<Pid, String>,
    /// Each `flip` step of this process, with its page and the page size.
    flips: Vec<(Step, Pid, usize)>,
    /// Each `flip` step done, with the reads of its page since.
    reading: Vec<(Step, OpLog)>,
    /// Each `damage` step of this process, with its page.
    planted: Vec<(Step, Pid)>,
    /// Each `lose` and `tear` step of this process, with the writes it acts on and its action.
    acting: Vec<(Step, OpLog, Planned)>,
    /// The `crash` step whose crash this process owes.
    crashing: Option<Step>,
}

impl State {
    pub(super) fn new(file: &str) -> State {
        State {
            file: file.to_owned(),
            roles: BTreeMap::new(),
            flips: Vec::new(),
            reading: Vec::new(),
            planted: Vec::new(),
            acting: Vec::new(),
            crashing: None,
        }
    }

    /// Returns the file each open opens.
    pub(super) fn file(&self) -> &str {
        &self.file
    }

    /// Returns the `crash` step whose crash the process that ended owed, and owes it no more.
    pub(super) fn take_crash(&mut self) -> Option<Step> {
        self.crashing.take()
    }

    /// Run a mark, fault, crash or probe step.
    pub(super) fn step(
        &mut self,
        rt: &SimRuntime,
        db: &Database<SimRuntime>,
        step: &Step,
    ) -> Result<(), Differs> {
        let t = db.testing();
        let failed = |what: &str, e: &dyn fmt::Debug| Differs::at(step, format!("{what}: {e:?}"));
        match &step.head {
            Head::Mark(marked) => {
                let mark = mark_of(t, marked).map_err(|e| Differs::at(step, e))?;
                t.plant(Plant::Mark(mark))
                    .map(drop)
                    .map_err(|e| failed("the mark is not set", &e))
            }
            Head::Fault { fault, page } => {
                let pid = page.pid(t).map_err(|e| Differs::at(step, e))?;
                self.roles.insert(pid, page.to_string());
                match fault {
                    Fault::Scribble => {
                        let named = pages::scribble(t, page, pid, &step.body)
                            .map_err(|e| Differs::at(step, e))?;
                        self.roles.extend(named);
                        Ok(())
                    }
                    Fault::Leak => {
                        let leaked = t
                            .leak(pid)
                            .map_err(|e| failed("no page is allocated", &e))?;
                        self.roles
                            .insert(leaked, format!("a page leaked beside {page}"));
                        Ok(())
                    }
                    fault => self
                        .fault(rt, t, step, *fault, pid)
                        .map_err(|e| failed("the fault is not planted", &e)),
                }
            }
            Head::Crash(at) => {
                if self.crashing.is_some() {
                    return Err(Differs::at(step, "a crash is planted already"));
                }
                crash(rt, at);
                self.crashing = Some(step.clone());
                Ok(())
            }
            Head::Probe(probed) => probe_rows(step, self.probe(rt, t, probed)),
            _ => unreachable!("state runs only marks, faults, crashes and probes"),
        }
    }

    fn fault(
        &mut self,
        rt: &SimRuntime,
        t: Testing<'_, SimRuntime>,
        step: &Step,
        fault: Fault,
        pid: Pid,
    ) -> Result<(), adtb::testing::Failed> {
        let size = t.probe().page_size();
        let offset = pid.0 * size as u64;
        let writes = || Match::kind(OpKind::Write).path(&self.file).covers(offset);
        match fault {
            Fault::Damage => {
                t.plant(Plant::Refuse {
                    page: pid,
                    damage: Damage::Checksum,
                })?;
                self.planted.push((step.clone(), pid));
            }
            Fault::Flip => self.flips.push((step.clone(), pid, size)),
            Fault::Lose => {
                let log = rt.logged(writes());
                self.acting.push((step.clone(), log, Planned::Lose));
                rt.extend(Plan::new().rule(writes(), Action::Lose));
            }
            Fault::Tear => {
                let log = rt.logged(writes());
                self.acting.push((step.clone(), log, Planned::Tear));
                let torn = Action::Tear(vec![true], Box::new(|| {}));
                rt.extend(Plan::new().first(writes(), torn));
            }
            Fault::Free => drop(t.plant(Plant::Free(pid))?),
            Fault::Scribble | Fault::Leak => unreachable!("an edit or a leak is no device fault"),
        }
        Ok(())
    }

    /// Returns a difference for each `lose` or `tear` of the process that ended that met no write.
    pub(super) fn unmet(&mut self) -> Vec<Differs> {
        self.planted.clear();
        let met = |(_, log, planned): &(Step, OpLog, Planned)| {
            log.entries().iter().any(|e| e.planned() == Some(*planned))
        };
        std::mem::take(&mut self.acting)
            .into_iter()
            .filter(|a| !met(a))
            .map(|(step, ..)| Differs::at(&step, "the fault meets no write"))
            .collect()
    }

    /// Returns a difference for each `damage` of this process no read has met, the database still
    /// open.
    pub(super) fn unmet_damage(&mut self, t: Testing<'_, SimRuntime>) -> Vec<Differs> {
        std::mem::take(&mut self.planted)
            .into_iter()
            .filter(|(_, pid)| !t.probe().refusal_met(*pid))
            .map(|(step, _)| Differs::at(&step, "no read meets the fault"))
            .collect()
    }

    /// Returns a difference for each `flip` no read of the file has met since it was done.
    pub(super) fn unread(&mut self) -> Vec<Differs> {
        std::mem::take(&mut self.reading)
            .into_iter()
            .chain(
                std::mem::take(&mut self.flips)
                    .into_iter()
                    .map(|(s, ..)| (s, OpLog::default())),
            )
            .filter(|(_, log)| log.entries().is_empty())
            .map(|(step, _)| Differs::at(&step, "no read meets the fault"))
            .collect()
    }

    fn probe(&self, rt: &SimRuntime, t: Testing<'_, SimRuntime>, probed: &Probed) -> Vec<String> {
        match probed {
            Probed::Marks => {
                let mut names: Vec<String> = t
                    .probe()
                    .marks()
                    .into_iter()
                    .map(|m| t.mark_name(m))
                    .collect();
                names.sort();
                names
            }
            Probed::Damage => {
                let records = t.probe().records();
                let page = |(pid, damage): (&Pid, &Damage)| {
                    format!("{}: {}", self.role(*pid), kind(damage))
                };
                let mut out: Vec<String> = records.pages().iter().map(page).collect();
                out.sort();
                out.extend(
                    records
                        .log()
                        .iter()
                        .map(|f| format!("log: {}", self.problem(f))),
                );
                out.extend(records.full().then(|| "the damage list is full".to_owned()));
                out
            }
            Probed::Files(prefix) => files(rt, prefix),
            Probed::Holding => pages::held(t, &self.roles),
            Probed::Catalog(engine) => t
                .catalog(*engine)
                .unwrap_or_else(|e| vec![format!("the catalog is not read: {e:?}")]),
            Probed::Heap {
                relation,
                ctor,
                pages,
            } => pages::laid(t, relation, ctor, *pages),
        }
    }

    /// Returns the role a fault named `pid` by.
    fn role(&self, pid: Pid) -> String {
        pages::role(&self.roles, pid)
    }

    /// Returns the problem of `finding`, a page it names by its role.
    fn problem(&self, finding: &LogFinding) -> String {
        match finding.problem {
            LogProblem::Unlogged { pid, .. } => format!("Unlogged {}", self.role(pid)),
            problem => format!("{problem:?}"),
        }
    }

    /// Invert a byte of the body of each page a `flip` named, on the device. Returns why one
    /// could not be flipped.
    pub(super) fn flip(&mut self, rt: &SimRuntime) -> Result<(), String> {
        let flips = std::mem::take(&mut self.flips);
        if flips.is_empty() {
            return Ok(());
        }
        let mut io = rt.io();
        let mut done = |op| complete(rt, &mut io, op);
        let Done::Opened { file, .. } = done(Op::Open {
            path: self.file.clone(),
            create: false,
        })?
        .0
        else {
            return Err("the open answers no file".into());
        };
        for (step, pid, size) in flips {
            let offset = pid.0 * size as u64;
            let reads = Match::kind(OpKind::Read).path(&self.file).covers(offset);
            let read = Op::Read {
                file,
                offset,
                buf: AlignedBuf::zeroed(size),
            };
            let (_, buf) = done(read)?;
            let mut bytes = buf.ok_or("a read returns its buffer")?.to_vec();
            bytes[FLIPPED] ^= 0xff;
            let buf = AlignedBuf::from_slice(&bytes);
            done(Op::Write { file, offset, buf })?;
            self.reading.push((step, rt.logged(reads)));
        }
        done(Op::Sync { file })?;
        done(Op::Close { file })?;
        Ok(())
    }

    /// Restore as `restore` states, the file of later opens the restored one when it succeeds.
    pub(super) fn restore(
        &mut self,
        rt: &SimRuntime,
        options: &OpenOptions,
        step: &Step,
        restore: &Restore,
    ) -> Result<(), Differs> {
        let point = point(restore.at.as_deref())
            .ok_or_else(|| Differs::at(step, "the restore point is no version or position"))?;
        let options = OpenOptions {
            create: None,
            ..options.clone()
        };
        let got = ArchiveTarget::path(&restore.source).and_then(|source| {
            restore_report_in(rt.clone(), (&source, None), &restore.to, point, &options)
        });
        let got = match got {
            Ok(r) => {
                self.file.clone_from(&restore.to);
                let dormant = |d: &adtb::Dormant| {
                    format!(
                        "dormant {{ id: {}, grantee: {:?}, privilege: {:?}, target: {:?} }}",
                        d.id, d.grantee, d.privilege, d.target
                    )
                };
                Ok(r.dormant.iter().map(dormant).collect())
            }
            Err(e) => Err(refused(&e)),
        };
        let want = restore
            .want
            .clone()
            .map_or_else(|| Ok(step.rows.clone()), Err);
        if got == want {
            return Ok(());
        }
        let differs = Differs::at(step, "the restore differs");
        Err(match (want, got) {
            (Ok(expected), Ok(actual)) => differs.rows(&expected, &actual),
            (want, got) => differs.values(shown(want), shown(got)),
        })
    }
}

/// Returns the variant of `damage`, without the log positions it names.
fn kind(damage: &Damage) -> String {
    match damage {
        Damage::Stale { .. } => "Stale".into(),
        Damage::LogUnproven { .. } => "LogUnproven".into(),
        Damage::Settled { .. } => "Settled".into(),
        Damage::Checksum | Damage::Unreadable | Damage::Shape(_) => format!("{damage:?}"),
    }
}

/// Run a `probe files` step, no database open.
pub(super) fn probe_files(rt: &SimRuntime, step: &Step, prefix: &str) -> Result<(), Differs> {
    probe_rows(step, files(rt, prefix))
}

fn files(rt: &SimRuntime, prefix: &str) -> Vec<String> {
    rt.files()
        .into_keys()
        .filter(|p| p.starts_with(prefix))
        .collect()
}

fn probe_rows(step: &Step, got: Vec<String>) -> Result<(), Differs> {
    if got == step.rows {
        return Ok(());
    }
    Err(Differs::at(step, "the probe differs").rows(&step.rows, &got))
}

fn shown(r: Result<Vec<String>, String>) -> String {
    r.map_or_else(|e| e, |rows| format!("[{}]", rows.join(", ")))
}

/// Returns the mark that names `marked` alone.
fn mark_of(t: Testing<'_, SimRuntime>, marked: &Marked) -> Result<Quarantine, String> {
    let object = match marked {
        Marked::Database => return Ok(Quarantine::Database),
        Marked::Heap { relation, ctor } => t.heap(relation, ctor),
        Marked::Index { relation, clause } => t.index(relation, clause),
    };
    let object = object.ok_or("no such object")?;
    t.own_mark(object).map_err(|e| format!("{e:?}"))
}

/// Plan the crash `at` states in this run.
fn crash(rt: &SimRuntime, at: &CrashAt) {
    let crasher = rt.clone();
    let action = Action::Run(Box::new(move || crasher.crash_after(Duration::ZERO)));
    match at {
        CrashAt::Op(kind, path) => {
            rt.extend(Plan::new().first(Match::kind(*kind).path(path), action));
        }
        CrashAt::After(n) => {
            let numbered = rt.numbered().kinds().len() as u64;
            rt.arm(Match::any().except(Match::kind(OpKind::Timer)));
            rt.extend(Plan::new().at(numbered + n, action));
        }
    }
}

/// Returns what `op` completed with, and its buffer, or the error it failed with.
fn complete(
    rt: &SimRuntime,
    io: &mut <SimRuntime as Runtime>::Io,
    op: Op,
) -> Result<(Done, Option<AlignedBuf>), String> {
    let what = format!("{:?}", op.kind());
    let ticket = io.submit(op);
    loop {
        if let Some(c) = io.poll() {
            assert_eq!(ticket, c.ticket, "the one op completes");
            return c
                .result
                .map(|d| (d, c.buf))
                .map_err(|e| format!("{what} fails: {e:?}"));
        }
        rt.park();
    }
}
