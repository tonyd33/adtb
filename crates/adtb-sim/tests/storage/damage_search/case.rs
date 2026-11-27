//! The faults a case injects, the write-outs it injects them at, and the run of one case: the
//! sequence with its fault, then every consumer.

use adtb::testing::{Pid, page};
use std::collections::{BTreeMap, BTreeSet};

use adtb::runtime::{IoError, OpKind};
use adtb_sim::{Action, Match, Plan as SimPlan, SimRuntime};
use adtb_sys::runtime::Faults;
use adtb_testkit::search::Files;

use super::consumers;
use super::judge::{self, Hit, Verdicts};
use super::ledger;
use super::world::{self, CREATING, Holdings, Ids, Kind, Op, Placed, Ran, Rel, Rows, WriteOut};

/// How a case damages its page, or after its write-out the log or a checkpoint. A fault of the
/// log names a write to the log of the run with no fault, by its index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Injection {
    /// The write-out reports success and never reaches the device.
    Lost,
    /// The write-out lands torn, some units its page changed landing and some not, and the
    /// process crashes at the next force of the data file.
    Torn,
    /// Every read of the page from the write-out on fails, once or for the rest of the case.
    Unreadable { lasting: bool },
    /// The write-out lands with one byte of the page's body inverted, so the copy on the device
    /// fails its checksum.
    Flipped,
    /// The log write reports success and never reaches the device.
    LogLost { write: usize },
    /// The first read of the log write's first block after the write fails.
    LogUnreadable { write: usize },
    /// The process crashes at a point of the checkpoint whose fold the log write carries.
    Crash { point: Fold, write: usize },
}

impl Injection {
    /// Returns whether the injection damages the page of its write-out.
    pub fn on_page(self) -> bool {
        matches!(
            self,
            Injection::Lost | Injection::Torn | Injection::Unreadable { .. } | Injection::Flipped
        )
    }

    /// Returns the write to the log the injection names, `None` for one of a page.
    pub fn log_write(self) -> Option<usize> {
        match self {
            Injection::LogLost { write }
            | Injection::LogUnreadable { write }
            | Injection::Crash { write, .. } => Some(write),
            Injection::Lost
            | Injection::Torn
            | Injection::Unreadable { .. }
            | Injection::Flipped => None,
        }
    }
}

/// The point of a checkpoint's fold a crash stops it at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Fold {
    /// The log write holding the checkpoint record and the change of the damage list fails.
    Log,
    /// The force of that write's segment fails.
    Force,
    /// The force of the file header naming the checkpoint fails.
    Header,
    /// The process crashes at the first force of the data file after the next write-out of the
    /// damage list's page.
    List,
}

pub const FOLDS: [Fold; 4] = [Fold::Log, Fold::Force, Fold::Header, Fold::List];

/// The fault of a cell: the injection, and for a lost write which incarnation it hits and
/// whether the log holding its evidence is recycled after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Fault {
    /// A lost write of an incarnation the sequence frees afterwards.
    LostEarlier,
    /// A lost write of the incarnation alive at the sequence's end.
    LostCurrent,
    /// A lost write the sequence recycles the log past.
    Recycled,
    Torn,
    UnreadableOnce,
    UnreadableLasting,
    Flipped,
}

pub const FAULTS: [Fault; 7] = [
    Fault::LostEarlier,
    Fault::LostCurrent,
    Fault::Recycled,
    Fault::Torn,
    Fault::UnreadableOnce,
    Fault::UnreadableLasting,
    Fault::Flipped,
];

impl Fault {
    pub fn injection(self) -> Injection {
        match self {
            Fault::LostEarlier | Fault::LostCurrent | Fault::Recycled => Injection::Lost,
            Fault::Torn => Injection::Torn,
            Fault::UnreadableOnce => Injection::Unreadable { lasting: false },
            Fault::UnreadableLasting => Injection::Unreadable { lasting: true },
            Fault::Flipped => Injection::Flipped,
        }
    }
}

/// A sequence and what its run with no fault showed: the write-outs it numbered and its writes
/// to the log, and the relation holding each page after each operation.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub seed: u64,
    pub ops: Vec<Op>,
    /// Whether a second thread vacuums each write's relation during the write.
    pub concurrent: bool,
    pub writes: Vec<WriteOut>,
    pub log: Vec<ledger::LogWrite>,
    /// The damage list's page.
    pub list: Option<Pid>,
    /// The owners the observing run read.
    pub ids: Ids,
    pub held: Vec<Holdings>,
    /// The first id not reserved and the commit log's truncation bound after each operation.
    pub clog: Vec<(u64, u64)>,
    /// The image each write-out of a commit log page wrote, by its index in `writes`.
    pub images: BTreeMap<usize, Vec<u8>>,
    /// Each open of the run: the write-outs submitted before it, the operation after which its
    /// commit log stands as it found it, and the operation its write-outs count as during.
    pub opens: Vec<(u64, usize, usize)>,
}

/// Returns the plan of `ops`: one run counting its write-outs, and one observing its holdings.
/// The observing run reads pages, and its write-outs are numbered differently.
pub fn plan(seed: u64, ops: Vec<Op>, concurrent: bool) -> Plan {
    let count = |_: &SimRuntime| {};
    let clean = world::run(seed, &ops, &count, false, concurrent, false);
    let observed = world::run(seed, &ops, &count, true, concurrent, false);
    Plan {
        seed,
        ops,
        concurrent,
        list: ledger::list_page(&clean.files),
        writes: clean.trace.writes,
        images: clean.trace.images,
        opens: clean.opens,
        log: clean.log,
        ids: observed.ids,
        held: observed.held,
        clog: observed.clog,
    }
}

impl Plan {
    /// Returns the kind of page write-out `at` writes, or `None` for one of the create or of a
    /// page of no kind the search counts.
    pub fn kind(&self, at: usize) -> Option<Kind> {
        let w = self.writes[at];
        if w.during == CREATING {
            return None;
        }
        world::class(w.kind, w.owner, &self.ids)
    }

    /// Returns the units of [`world::UNIT`] bytes of its write that a tear of write-out `at`
    /// lands: of the units its page changed, a subset drawn from the seed and `at` with at least
    /// one landing and one not; every other unit of the write. Returns `None` when the page
    /// changed in fewer than two units, which a tear lands whole or not at all.
    pub fn tear(&self, at: usize) -> Option<Vec<bool>> {
        let changed = self.writes[at].changed;
        if changed.count_ones() < 2 {
            return None;
        }
        let (index, pages) = self.place(at);
        let draw = self.draw(at);
        let subsets = (1_u64 << changed.count_ones()) - 2;
        let mut pick = 1 + draw % subsets;
        let units = world::PAGE / world::UNIT;
        let mut mask = vec![true; pages * units];
        for u in (0..units).filter(|u| changed & (1 << u) != 0) {
            mask[index * units + u] = pick & 1 == 1;
            pick >>= 1;
        }
        Some(mask)
    }

    /// Returns the byte of its write that a flip of write-out `at` inverts: one past the page
    /// header, drawn from the seed and `at`.
    pub fn flip(&self, at: usize) -> usize {
        let (index, _) = self.place(at);
        let body = world::PAGE - page::HEADER_LEN;
        index * world::PAGE + page::HEADER_LEN + (self.draw(at) % body as u64) as usize
    }

    /// Returns the index of write-out `at`'s page in the write that carries it, and the pages that
    /// write carries.
    fn place(&self, at: usize) -> (usize, usize) {
        let writes = &self.writes;
        let same = |x: &&WriteOut| x.op == writes[at].op;
        let index = writes[..at].iter().rev().take_while(same).count();
        (index, index + writes[at..].iter().take_while(same).count())
    }

    /// Returns a number drawn from the seed and write-out `at`.
    fn draw(&self, at: usize) -> u64 {
        let golden = 0x9e37_79b9_7f4a_7c15_u64;
        (self.seed.wrapping_mul(golden) ^ at as u64).wrapping_mul(golden) >> 32
    }

    /// Returns whether `injection` at write-out `at` can take: a tear only of a page it splits.
    pub fn takes(&self, at: usize, injection: Injection) -> bool {
        injection != Injection::Torn || self.tear(at).is_some()
    }

    /// Returns the user relation the page write-out `at` writes belongs to, by where the
    /// observing run placed it.
    pub fn owner(&self, at: usize) -> Option<Rel> {
        self.kind(at)?;
        self.placed_written(self.writes[at])?.1
    }

    /// Returns where the observing run placed page `pid` after operation `op`, after the last for
    /// an operation past it; `None` for a page free then.
    pub fn placed(&self, pid: Pid, op: usize) -> Option<Placed> {
        let last = self.held.len().checked_sub(1)?;
        self.held[op.min(last)].get(&pid).copied()
    }

    /// Returns whether `placed` lists the incarnation of the page `w` writes: a structure listed by
    /// its object whose pages carry `w`'s owner, or the one structure listed by no object that
    /// holds a page of `w`'s kind.
    fn lists(&self, placed: Placed, w: WriteOut) -> bool {
        match placed.2 {
            Some(owner) => owner == w.owner,
            None => world::class(w.kind, w.owner, &self.ids)
                .is_some_and(|kind| kind.holds_unlisted(placed.0.0)),
        }
    }

    /// Returns where the observing run placed the incarnation of the page `w` writes, as the
    /// operation nearest `w`'s that lists that incarnation lists it, else the one structure that
    /// holds every page of `w`'s kind; `None` when neither places it.
    pub fn placed_written(&self, w: WriteOut) -> Option<Placed> {
        let op = w.during.min(self.held.len().checked_sub(1)?);
        let mut ats: Vec<usize> = (0..self.held.len()).collect();
        ats.sort_by_key(|at| (at.abs_diff(op), *at));
        ats.into_iter()
            .find_map(|at| {
                self.held[at]
                    .get(&w.pid)
                    .copied()
                    .filter(|placed| self.lists(*placed, w))
            })
            .or_else(|| self.sole(w))
    }

    /// Returns the placement of a page of the one structure that holds every page of `w`'s kind.
    fn sole(&self, w: WriteOut) -> Option<Placed> {
        let held = world::class(w.kind, w.owner, &self.ids)?.sole_holder()?;
        Some(((held, None), None, None))
    }

    /// Returns where the observing run placed page `pid` after operation `op`, after the last for
    /// an operation past it: [`world::FREE`] for a page free then. When it lists another
    /// incarnation than the last write-out of the page up to then wrote, returns the one structure
    /// that holds every page of that write-out's kind, `None` for a kind that has none.
    pub fn placed_at(&self, pid: Pid, op: usize) -> Option<Placed> {
        let last = self.held.len().checked_sub(1)?;
        let Some(placed) = self.held[op.min(last)].get(&pid).copied() else {
            return Some(world::FREE);
        };
        let written = self
            .writes
            .iter()
            .rev()
            .filter(|w| w.during == CREATING || op >= last || w.during <= op)
            .find(|w| w.pid == pid)?;
        if self.lists(placed, *written) {
            Some(placed)
        } else {
            self.sole(*written)
        }
    }

    /// Returns the cell's fault an `injection` at write-out `at` makes.
    ///
    /// Precondition: the injection damages the page of its write-out.
    pub fn fault(&self, at: usize, injection: Injection) -> Fault {
        match injection {
            Injection::LogLost { .. }
            | Injection::LogUnreadable { .. }
            | Injection::Crash { .. } => {
                unreachable!("only an injection on a page has a cell")
            }
            Injection::Torn => Fault::Torn,
            Injection::Unreadable { lasting: false } => Fault::UnreadableOnce,
            Injection::Unreadable { lasting: true } => Fault::UnreadableLasting,
            Injection::Flipped => Fault::Flipped,
            Injection::Lost => {
                let w = self.writes[at];
                let after = (w.during + 1).min(self.ops.len());
                if self.ops[after..].contains(&Op::Recycle)
                    || self.ops.get(w.during) == Some(&Op::Recycle)
                {
                    Fault::Recycled
                } else if self.freed_after(w) {
                    Fault::LostEarlier
                } else {
                    Fault::LostCurrent
                }
            }
        }
    }

    /// Returns whether the incarnation `w` writes is freed later in the sequence.
    fn freed_after(&self, w: WriteOut) -> bool {
        self.freed_by(w).is_some()
    }

    /// Returns the operation after which the incarnation `w` writes is first free, if the
    /// sequence frees it.
    fn freed_by(&self, w: WriteOut) -> Option<usize> {
        let held = &self.held;
        if w.during >= held.len() {
            return None;
        }
        let before = w.during.checked_sub(1).map(|i| &held[i]);
        let alive =
            before.is_some_and(|h| h.contains_key(&w.pid)) || held[w.during].contains_key(&w.pid);
        let freed = (w.during..held.len()).find(|&i| !held[i].contains_key(&w.pid));
        freed.filter(|_| alive)
    }

    /// Returns the last write-out of the page `w` writes, when the sequence frees `w`'s
    /// incarnation and writes the page again after; else `w`.
    pub fn incarnation_at_end(&self, w: WriteOut) -> WriteOut {
        let Some(freed) = self.freed_by(w) else {
            return w;
        };
        let mut writes = self.writes.iter().rev().copied();
        writes
            .find(|x| x.pid == w.pid && x.during != CREATING && x.during > freed)
            .unwrap_or(w)
    }
}

/// One case: a plan, the write-out to inject at, and the injection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub at: usize,
    pub injection: Injection,
}

/// Plan the faults of `points` on `rt`, each at the write of its write-out in `plan`. At most one of
/// them is no lost write.
fn arm(rt: &SimRuntime, plan: &Plan, points: &[Point]) {
    let failing = points.iter().filter(|p| p.injection != Injection::Lost);
    assert!(failing.count() <= 1, "at most one point plans failures");
    let mut planned = SimPlan::new();
    for point in points {
        let w = plan.writes[point.at];
        let action = match point.injection {
            Injection::Crash {
                point: Fold::List, ..
            } => planning(rt, SimPlan::new().first(data_force(), failed())),
            Injection::LogLost { write }
            | Injection::LogUnreadable { write }
            | Injection::Crash { write, .. } => after(rt, plan, point.injection, write),
            Injection::Flipped => Action::Flip(plan.flip(point.at)),
            Injection::Lost | Injection::Torn | Injection::Unreadable { .. } => {
                injected(rt, point.injection, w.pid, plan.tear(point.at))
            }
        };
        planned = planned.at(w.op, action);
    }
    rt.plan(planned);
}

/// Returns the action that plans, at its write-out, `injection` at the log write `write` of the
/// run with no fault in `plan`.
fn after(rt: &SimRuntime, plan: &Plan, injection: Injection, write: usize) -> Action {
    let w = &plan.log[write];
    let at = Match::kind(OpKind::Write).path(&w.path).offset(w.offset);
    let failing = |ops: Match| planning(rt, SimPlan::new().first(ops, failed()));
    let then = match injection {
        Injection::LogLost { .. } => Action::Lose,
        Injection::LogUnreadable { .. } => {
            failing(Match::kind(OpKind::Read).path(&w.path).covers(w.offset))
        }
        Injection::Crash {
            point: Fold::Log, ..
        } => failed(),
        Injection::Crash {
            point: Fold::Force, ..
        } => failing(Match::kind(OpKind::Sync).path(&w.path)),
        Injection::Crash {
            point: Fold::Header,
            ..
        } => failing(data_force()),
        Injection::Crash {
            point: Fold::List, ..
        }
        | Injection::Lost
        | Injection::Torn
        | Injection::Unreadable { .. }
        | Injection::Flipped => {
            unreachable!(
                "a fault of a page, or a crash past the list's write-out, is planned there"
            )
        }
    };
    planning(rt, SimPlan::new().first(at, then))
}

fn failed() -> Action {
    Action::Fail(IoError::Failed)
}

fn data_force() -> Match {
    world::on_data(Match::kind(OpKind::Sync))
}

/// Returns the action that adds `then`, lasting, to the plan of `rt`.
fn planning(rt: &SimRuntime, then: SimPlan) -> Action {
    let rt = rt.clone();
    Action::Run(Box::new(move || rt.extend(then.lasting())))
}

/// Returns the action that injects `injection` at its write, planning there the ops that fail
/// after it for the page `pid`. A tear lands the units of the write `tear` sets.
fn injected(rt: &SimRuntime, injection: Injection, pid: Pid, tear: Option<Vec<bool>>) -> Action {
    let rt = rt.clone();
    match injection {
        Injection::Lost => Action::Lose,
        Injection::Torn => Action::Tear(
            tear.expect("a torn write-out's page changed in more than one unit"),
            Box::new(move || {
                rt.set_faults(Faults {
                    torn_p: 1.0,
                    ..Faults::none(world::UNIT as u64)
                });
                let forces = world::on_data(Match::kind(OpKind::Sync));
                let failed = Action::Fail(IoError::Failed);
                rt.extend(SimPlan::new().rule(forces, failed).lasting());
            }),
        ),
        Injection::LogLost { .. } | Injection::LogUnreadable { .. } | Injection::Crash { .. } => {
            unreachable!("a fault of the log is planned at its write to the log")
        }
        Injection::Flipped => unreachable!("a flip is planned at its write-out"),
        Injection::Unreadable { lasting } => Action::Run(Box::new(move || {
            let reads = world::on_data(Match::kind(OpKind::Read))
                .covers(world::DATA.range(pid).start as u64);
            let failed = Action::Fail(IoError::Failed);
            let then = if lasting {
                SimPlan::new().rule(reads, failed)
            } else {
                SimPlan::new().first(reads, failed)
            };
            rt.extend(then.lasting());
        })),
    }
}

/// What one case showed, for the judge and the floor.
#[derive(Debug)]
pub struct Outcome {
    /// The errors of the operations of the faulted run, by their index.
    pub failed: BTreeMap<usize, String>,
    /// The writes of the faulted run a vacuum on a second thread ran during.
    pub overlaps: usize,
    /// Whether the consumers ran their open of the faulted file.
    pub consumed: bool,
    /// Whether a planned failure of a read fired in the faulted run.
    pub read_failed: bool,
    /// Whether the consumers' open of the faulted file succeeded.
    pub opened: bool,
    /// Whether a process after the first a planned read failure struck in had one too.
    pub failed_after_restart: bool,
    /// Whether the faulted run of a pair ended with a page torn whose retained log holds an image
    /// of it after a settled record of it.
    pub torn_settle: bool,
    /// Whether the faulted run left the first fault's page blank in the file.
    pub blank: bool,
    pub verdicts: Verdicts,
}

/// Run the case `point` of `plan`, then every consumer, and judge them. Returns `None` when the
/// injection did not take: the write-out never came, or a tear landed whole.
pub fn run(plan: &Plan, point: Point) -> Option<Outcome> {
    run_all(plan, &[point])
}

/// Run the case of a lost write at `first` and the fault of `second`, then every consumer, and
/// judge them. Returns `None` when a fault did not take.
pub fn run_pair(plan: &Plan, first: Point, second: Point) -> Option<Outcome> {
    run_all(plan, &[first, second])
}

/// Run `plan` with the faults of `points`, at most one of them no lost write, then every
/// consumer, and judge them with the first point's cell first. A pair is judged also on the
/// evidence its file held after each process.
fn run_all(plan: &Plan, points: &[Point]) -> Option<Outcome> {
    let mut hits = Vec::new();
    for p in points.iter().filter(|p| p.injection.on_page()) {
        hits.push(Hit {
            written: plan.writes[p.at],
            kind: plan.kind(p.at)?,
            fault: plan.fault(p.at, p.injection),
        });
    }
    let paired = points.len() > 1;
    let ran = faulted(plan, points, paired)?;
    for (p, h) in points.iter().filter(|p| p.injection.on_page()).zip(&hits) {
        let pid = h.written.pid;
        if p.injection == Injection::Torn && sealed(pid, &world::page(&ran.files, pid)) {
            return None;
        }
    }
    let log = points
        .iter()
        .find(|p| !p.injection.on_page())
        .map(|p| p.injection);
    if let Some(injection) = log
        && !took(plan, &ran, injection)
    {
        return None;
    }
    let lasting = points
        .iter()
        .filter(|p| p.injection.on_page())
        .zip(&hits)
        .find(|(p, _)| p.injection == (Injection::Unreadable { lasting: true }));
    let pid = lasting.map_or(hits[0].written.pid, |(_, h)| h.written.pid);
    let started = version(plan, &ran);
    let lost_log = match log {
        Some(Injection::LogLost { .. }) => lost(&ran),
        _ => None,
    };
    let allowed = allowed(&ran, lost_log.as_ref().map(|_| kept(&ran)));
    let keys = keys(&allowed);
    let damaged: Vec<Pid> = hits.iter().map(|h| h.written.pid).collect();
    let observed = consumers::run(
        plan.seed,
        &ran,
        pid,
        lasting.is_some(),
        started,
        &keys,
        &damaged,
    );
    let evidence = paired.then(|| {
        let mut snapshots = ran.evidence.clone();
        snapshots.extend(ledger::evidence(&observed.files));
        let ended = ran
            .log
            .iter()
            .filter(|w| !w.dropped)
            .flat_map(|w| w.ended.iter().copied())
            .collect();
        (snapshots, ended)
    });
    let verdicts = judge::judge(&judge::Case {
        plan,
        ran: &ran,
        hits,
        allowed: &allowed,
        started,
        observed: &observed,
        lost_log,
        evidence: evidence.as_ref().map(|(s, e)| (s.as_slice(), e)),
    });
    Some(Outcome {
        failed: ran.failed,
        overlaps: ran.overlaps,
        consumed: observed.open.is_some(),
        read_failed: ran.read_failed,
        opened: matches!(observed.open, Some(Ok(()))),
        failed_after_restart: ran
            .failed_reads
            .iter()
            .position(|n| *n > 0)
            .is_some_and(|first| ran.failed_reads[first + 1..].iter().any(|n| *n > 0)),
        torn_settle: paired && torn_settle(&ran.files),
        blank: world::page(&ran.files, damaged[0]).iter().all(|b| *b == 0),
        verdicts,
    })
}

/// Returns whether `files` hold a page whose copy fails its checksum and whose retained log holds
/// an image of it after a settled record of it.
fn torn_settle(files: &Files) -> bool {
    let Some(e) = ledger::evidence(files) else {
        return false;
    };
    let imaged = |pid: Pid, at: u64| {
        e.ended
            .iter()
            .any(|&(p, l, image)| p == pid && l > at && image)
    };
    e.records
        .iter()
        .filter(|(_, _, kind)| *kind == ledger::SETTLED)
        .any(|&(pid, at, _)| imaged(pid, at) && !sealed(pid, &world::page(files, pid)))
}

/// Returns the write to the log a plan made the device lose in `ran`.
fn lost(ran: &Ran) -> Option<judge::LostLog> {
    let at = ran.log.iter().position(|w| w.planned)?;
    let w = &ran.log[at];
    let tail = ran.log[at + 1..]
        .first()
        .is_none_or(|next| next.lsn <= w.lsn);
    let named = ledger::checkpoint(&ran.files).is_some_and(|c| c >= w.lsn);
    let end = ran
        .processes
        .iter()
        .find(|(submitted, _)| *submitted > w.submitted)
        .map_or(u64::MAX, |(_, outs)| *outs);
    Some(judge::LostLog {
        lsn: w.lsn,
        tail,
        named,
        after: w.outs..end,
    })
}

/// Returns the index of the last state `ran` acknowledged before it submitted the write to the
/// log a plan made the device lose.
fn kept(ran: &Ran) -> usize {
    let Some(lost) = ran.log.iter().find(|w| w.planned) else {
        return 0;
    };
    ran.acked
        .submitted
        .iter()
        .filter(|s| **s <= lost.submitted)
        .count()
}

/// Returns whether `injection`, a fault of the log, took in `ran`: a plan acted at its log write,
/// and for a crash, that write still carries a fold and the process crashed. A crash at the
/// write-out of the damage list acts at that write-out, past its log write.
fn took(plan: &Plan, ran: &Ran, injection: Injection) -> bool {
    let Some(write) = injection.log_write() else {
        return true;
    };
    let w = &plan.log[write];
    let list = matches!(
        injection,
        Injection::Crash {
            point: Fold::List,
            ..
        }
    );
    let acted = ran
        .log
        .iter()
        .find(|x| x.path == w.path && x.offset == w.offset && (x.planned || list));
    match injection {
        Injection::Crash { .. } => acted.is_some_and(|x| x.folds) && ran.cut.is_some(),
        _ => acted.is_some(),
    }
}

/// Returns the keys to read by key from each relation: up to 12 of those any allowed state
/// holds, spread over them, and one no write gave.
fn keys(allowed: &[Rows]) -> BTreeMap<Rel, Vec<i64>> {
    let mut out: BTreeMap<Rel, Vec<i64>> = BTreeMap::new();
    for rel in [Rel::Users, Rel::Docs, Rel::Tags] {
        let all: BTreeSet<i64> = allowed
            .iter()
            .filter_map(|s| s.get(&rel))
            .flat_map(|r| r.keys().copied())
            .collect();
        let all: Vec<i64> = all.into_iter().collect();
        let mut picked: Vec<i64> = if all.len() <= 12 {
            all.clone()
        } else {
            (0..12).map(|i| all[i * (all.len() - 1) / 11]).collect()
        };
        picked.push(1_000_000);
        out.insert(rel, picked);
    }
    out
}

/// Returns the run of `plan` with the faults of `points`, reading its file's evidence after each
/// process when `watched`, or `None` when a write-out of one of them never came as it did in the
/// run with no fault.
fn faulted(plan: &Plan, points: &[Point], watched: bool) -> Option<Ran> {
    let arming = |rt: &SimRuntime| arm(rt, plan, points);
    let ran = world::run(
        plan.seed,
        &plan.ops,
        &arming,
        false,
        plan.concurrent,
        watched,
    );
    let reached = |p: &Point| {
        let w = plan.writes[p.at];
        ran.trace.writes.get(p.at).is_some_and(|got| *got == w)
    };
    points.iter().all(reached).then_some(ran)
}

/// Returns whether `page` carries the checksum it has as page `pid`: whether a tear of it landed
/// whole or not at all.
pub fn sealed(pid: Pid, page: &[u8]) -> bool {
    let mut copy = page.to_vec();
    page::seal(pid, &mut copy);
    copy == page
}

/// Returns the schema version the consumers open with: the last one the sequence started that the
/// whole-database mark did not refuse.
fn version(plan: &Plan, ran: &Ran) -> u8 {
    let end = ran.cut.map_or(plan.ops.len(), |c| c + 1);
    let started = plan.ops[..end]
        .iter()
        .enumerate()
        .filter(|(i, o)| **o == Op::Migrate && !ran.refused_under_mark.contains(i))
        .count();
    (started as u8).min(world::LAST)
}

/// Returns the states the file may hold: the last acknowledged one, and when a crash cut the
/// operation that was committing, that operation's too. With a write to the log lost, every
/// acknowledged state from the one at index `kept` on.
fn allowed(ran: &Ran, kept: Option<usize>) -> Vec<Rows> {
    let last = ran.acked.last().clone();
    let mut out = match kept {
        Some(from) => ran.acked.states[from..].to_vec(),
        None => vec![last.clone()],
    };
    if let Some(op) = &ran.pending {
        out.push(world::applied(last, op));
    }
    out
}
