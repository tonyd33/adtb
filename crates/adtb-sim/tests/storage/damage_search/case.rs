//! The faults a case injects, the write-outs it injects them at, and the run of one case: the
//! sequence with its fault, then every consumer.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use adtb::runtime::{IoError, Op as IoOp};
use adtb::storage::{self, Pid};
use adtb_simkit::search::{Action, Hooked};
use adtb_sys::runtime::Faults;

use super::consumers;
use super::judge::{self, Hit, Verdicts};
use super::world::{self, CREATING, Kind, Op, PAGE, Ran, Rel, Rows, Traced, WriteOut};

/// How a case damages its page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Injection {
    /// The write-out reports success and never reaches the device.
    Lost,
    /// The write-out lands torn, and the process crashes at the next force of the data file.
    Torn,
    /// Every read of the page from the write-out on fails, once or for the rest of the case.
    Unreadable { lasting: bool },
}

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
}

pub const FAULTS: [Fault; 6] = [
    Fault::LostEarlier,
    Fault::LostCurrent,
    Fault::Recycled,
    Fault::Torn,
    Fault::UnreadableOnce,
    Fault::UnreadableLasting,
];

impl Fault {
    pub fn injection(self) -> Injection {
        match self {
            Fault::LostEarlier | Fault::LostCurrent | Fault::Recycled => Injection::Lost,
            Fault::Torn => Injection::Torn,
            Fault::UnreadableOnce => Injection::Unreadable { lasting: false },
            Fault::UnreadableLasting => Injection::Unreadable { lasting: true },
        }
    }
}

/// A sequence and what its run with no fault showed: the write-outs it numbered, and the
/// relation holding each page after each operation.
#[derive(Debug)]
pub struct Plan {
    pub seed: u64,
    pub ops: Vec<Op>,
    /// Whether a second thread vacuums each write's relation during the write.
    pub concurrent: bool,
    pub clean: Ran,
    pub observed: Ran,
}

/// Returns the plan of `ops`: one run counting its write-outs, and one observing its holdings.
/// The observing run reads pages, and its write-outs are numbered differently.
pub fn plan(seed: u64, ops: Vec<Op>, concurrent: bool) -> Plan {
    let count = |h: &Hooked, t: &Traced| {
        let t = t.clone();
        h.arm(move |op, path| world::write_out(&t, op, path));
    };
    let clean = world::run(seed, &ops, &count, false, concurrent);
    let observed = world::run(seed, &ops, &count, true, concurrent);
    Plan {
        seed,
        ops,
        concurrent,
        clean,
        observed,
    }
}

impl Plan {
    /// Returns the kind of page write-out `at` writes, or `None` for one of the create or of a
    /// page of no kind the search counts.
    pub fn kind(&self, at: usize) -> Option<Kind> {
        let w = self.clean.trace.writes[at];
        if w.during == CREATING {
            return None;
        }
        world::class(w.kind, w.owner, &self.observed.ids)
    }

    /// Returns the user relation the page write-out `at` writes belongs to.
    pub fn owner(&self, at: usize) -> Option<Rel> {
        let w = self.clean.trace.writes[at];
        self.observed.ids.owner(self.kind(at)?, w.owner)
    }

    /// Returns the cell's fault an `injection` at write-out `at` makes.
    pub fn fault(&self, at: usize, injection: Injection) -> Fault {
        match injection {
            Injection::Torn => Fault::Torn,
            Injection::Unreadable { lasting: false } => Fault::UnreadableOnce,
            Injection::Unreadable { lasting: true } => Fault::UnreadableLasting,
            Injection::Lost => {
                let w = self.clean.trace.writes[at];
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
        let held = &self.observed.held;
        if w.during >= held.len() {
            return false;
        }
        let before = w.during.checked_sub(1).map(|i| &held[i]);
        let alive =
            before.is_some_and(|h| h.contains_key(&w.pid)) || held[w.during].contains_key(&w.pid);
        alive && held[w.during..].iter().any(|h| !h.contains_key(&w.pid))
    }

    /// Returns whether `pid` is allocated at the sequence's end, by the observing run.
    pub fn held_at_end(&self, pid: Pid) -> bool {
        self.observed
            .held
            .last()
            .is_some_and(|h| h.contains_key(&pid))
    }
}

/// One case: a plan, the write-out to inject at, and the injection.
#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub at: usize,
    pub injection: Injection,
}

/// Set the faults of `points` on `hook`, numbering write-outs into `trace`, each at the write of
/// its write-out in `plan`. At most one of them is no lost write.
fn arm(hook: &Hooked, trace: &Traced, plan: &Plan, points: &[Point]) {
    let t = trace.clone();
    hook.arm(move |op, path| world::write_out(&t, op, path));
    for (i, point) in points.iter().enumerate() {
        let w = plan.clean.trace.writes[point.at];
        let action = injected(hook, point.injection, w.pid);
        if i == 0 {
            hook.plan(w.op, action);
        } else {
            hook.plan_also(w.op, action);
        }
    }
}

/// Returns the action that injects `injection` at its write, setting on `hook` the failures
/// that follow it for the page `pid`.
fn injected(hook: &Hooked, injection: Injection, pid: Pid) -> Action {
    let on = Arc::new(AtomicU64::new(0));
    match injection {
        Injection::Lost => Action::Lose,
        Injection::Torn => {
            let rt = hook.rt.clone();
            let armed = on.clone();
            hook.fail_named(move |op, path| {
                let force = matches!(op, IoOp::Sync { .. }) && world::is_data(path);
                (force && on.load(Ordering::SeqCst) == 1).then_some(IoError::Failed)
            });
            Action::Run(Box::new(move || {
                rt.set_faults(Faults {
                    torn_p: 1.0,
                    ..Faults::none(512)
                });
                armed.store(1, Ordering::SeqCst);
            }))
        }
        Injection::Unreadable { lasting } => {
            let armed = on.clone();
            hook.fail_named(move |op, path| unreadable(op, path, pid, lasting, &on));
            Action::Run(Box::new(move || armed.store(1, Ordering::SeqCst)))
        }
    }
}

/// Returns the failure of a read of `pid` while `on` counts it armed: 1 armed, 2 spent.
fn unreadable(
    op: &IoOp,
    path: Option<&str>,
    pid: Pid,
    lasting: bool,
    on: &AtomicU64,
) -> Option<IoError> {
    let IoOp::Read { offset, buf, .. } = op else {
        return None;
    };
    let at = pid.0 * PAGE as u64;
    let covers = *offset <= at && at < offset + buf.len() as u64;
    if !covers || !world::is_data(path) || on.load(Ordering::SeqCst) != 1 {
        return None;
    }
    if !lasting {
        on.store(2, Ordering::SeqCst);
    }
    Some(IoError::Failed)
}

/// What one case showed, for the judge and the floor.
#[derive(Debug)]
pub struct Outcome {
    /// The errors of the operations of the faulted run, by their index.
    pub failed: BTreeMap<usize, String>,
    /// The writes of the faulted run a vacuum on a second thread ran during.
    pub overlaps: usize,
    /// Whether the consumers' open of the faulted file succeeded.
    pub opened: bool,
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
/// consumer, and judge them with the first point's cell first.
fn run_all(plan: &Plan, points: &[Point]) -> Option<Outcome> {
    let mut hits = Vec::new();
    for p in points {
        hits.push(Hit {
            written: plan.clean.trace.writes[p.at],
            kind: plan.kind(p.at)?,
            fault: plan.fault(p.at, p.injection),
        });
    }
    let ran = faulted(plan, points)?;
    for (p, h) in points.iter().zip(&hits) {
        let pid = h.written.pid;
        if p.injection == Injection::Torn && sealed(pid, &world::page(&ran.files, pid)) {
            return None;
        }
    }
    let lasting = points
        .iter()
        .zip(&hits)
        .find(|(p, _)| p.injection == (Injection::Unreadable { lasting: true }));
    let pid = lasting.map_or(hits[0].written.pid, |(_, h)| h.written.pid);
    let started = version(plan, &ran);
    let allowed = allowed(&ran);
    let keys = keys(&allowed);
    let observed = consumers::run(plan.seed, &ran, pid, lasting.is_some(), started, &keys);
    let verdicts = judge::judge(&judge::Case {
        plan,
        ran: &ran,
        hits,
        allowed: &allowed,
        started,
        observed: &observed,
    });
    Some(Outcome {
        failed: ran.failed,
        overlaps: ran.overlaps,
        opened: matches!(observed.open, Some(Ok(()))),
        verdicts,
    })
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

/// Returns the run of `plan` with the faults of `points`, or `None` when a write-out of one of
/// them never came as it did in the run with no fault.
fn faulted(plan: &Plan, points: &[Point]) -> Option<Ran> {
    let arming = |h: &Hooked, t: &Traced| arm(h, t, plan, points);
    let ran = world::run(plan.seed, &plan.ops, &arming, false, plan.concurrent);
    let reached = |p: &Point| {
        let w = plan.clean.trace.writes[p.at];
        ran.trace.writes.get(p.at).is_some_and(|got| *got == w)
    };
    points.iter().all(reached).then_some(ran)
}

/// Returns whether `page` carries the checksum it has as page `pid`: whether a tear of it landed
/// whole or not at all.
fn sealed(pid: Pid, page: &[u8]) -> bool {
    let mut copy = page.to_vec();
    storage::seal(pid, &mut copy);
    copy == page
}

/// Returns the schema version the consumers open with: the last one the sequence started.
fn version(plan: &Plan, ran: &Ran) -> u8 {
    let end = ran.cut.map_or(plan.ops.len(), |c| c + 1);
    let started = plan.ops[..end]
        .iter()
        .filter(|o| **o == Op::Migrate)
        .count();
    (started as u8).min(world::LAST)
}

/// Returns the states the file may hold: the last acknowledged one, and when a crash cut the
/// operation that was committing, that operation's too.
fn allowed(ran: &Ran) -> Vec<Rows> {
    let last = ran.acked.last().clone();
    let mut out = vec![last.clone()];
    if let Some(op) = &ran.pending {
        out.push(world::applied(last, op));
    }
    out
}
