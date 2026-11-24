//! The judgement of one case from the test's own model: the rows the sequence acknowledged, the
//! pages the faults hit and the relation each write-out names. A case ends with the data intact,
//! with the damage reported and marked as the mapping of damage to marks says, or with a storage
//! failure the contract allows; anything else is a violation.

use adtb::testing::{Pid, Quarantine, page};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use adtb::Proof;

use super::case::{self, Fault, Plan};
use super::consumers::{Backup, Consumer, Copy, Dumped, Finding, Lookups, Observed, Read};
use super::ledger::{self, Ended, Evidence, Record, SETTLED};
use super::stated::{self, Class, Held, Holding, PHASES, Phase, Stated};
use super::world::{self, Kind, Op, Ran, Rel, Rows, WriteOut};

/// One way a case broke the contract, at the consumer that showed it. `what` names the shape
/// alone, the same across sequences; `detail` holds what this case saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    pub consumer: Consumer,
    pub what: String,
    pub detail: String,
}

/// How a case ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ending {
    Intact,
    Reported,
    /// Verify names a stale page or the page is stale, and a mark it maps to is missing.
    Unmarked,
    StorageFailure,
}

#[derive(Debug)]
pub struct Verdicts {
    pub violations: Vec<Violation>,
    /// The rows of the table the case asked.
    pub rows: BTreeSet<Row>,
    /// The consumers whose judgement ran and could meet the first fault's page.
    pub reached: BTreeSet<Consumer>,
    /// The consumers whose answer carries the first fault's page: the open recorded or settled
    /// it, a read was refused or failed, verify named it, the backup or its proof failed, the
    /// dump failed or its copy refused a read.
    pub met: BTreeSet<Consumer>,
    /// The damage verify's findings give the first fault's page, by name: `Stale`, `Settled`,
    /// `Checksum`.
    pub named: Vec<String>,
    pub ending: Ending,
    /// The damage records and the durable marks the file's evidence held, for a case judged on
    /// it.
    pub evidence: (usize, usize),
    /// Whether the judge compared where the plan placed a damaged page with the mapping's walk.
    pub walked: bool,
    /// Whether the judge compared whether the commit log settled a status page with what the plan
    /// and the retained log say.
    pub settled: bool,
}

/// One fault of a case: the write-out it hit, the class of its page and the fault.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub written: WriteOut,
    pub kind: Kind,
    pub fault: Fault,
}

pub struct Case<'a> {
    pub plan: &'a Plan,
    pub ran: &'a Ran,
    /// The faults, the one whose cell the case counts for first.
    pub hits: Vec<Hit>,
    pub allowed: &'a [Rows],
    /// The schema version of the last migration the sequence started, which a crash may have
    /// cut before its open returned.
    pub started: u8,
    pub observed: &'a Observed,
    /// The write to the log the device lost.
    pub lost_log: Option<LostLog>,
    /// What the file held after each process of the run and after the consumers, in order, with
    /// every free and image the run logged, for a case judged on its evidence.
    pub evidence: Option<(&'a [Evidence], &'a BTreeSet<Ended>)>,
}

/// A write to the log the device lost: the LSN it starts at, whether it was the log's last
/// batch, no later write of the run starting past it, whether the file header names a
/// checkpoint at or past it, and the numbers of the write-outs its process submitted after it.
#[derive(Clone, Debug)]
pub struct LostLog {
    pub lsn: u64,
    pub tail: bool,
    pub named: bool,
    pub after: Range<u64>,
}

/// What the model says of one damaged page.
struct Page {
    pid: Pid,
    kind: Kind,
    fault: Fault,
    /// The user relation the observing run placed the page in when it was written.
    rel: Option<Rel>,
    /// Where the observing run placed the incarnation the faulted run left at its end, for a page
    /// the faulted run wrote out as the plan's run did and that is not free then.
    at_end: Option<Held>,
    /// Whether the commit log settles the page, by the plan and the retained log, for a status
    /// page that lost a write and whose first open after the loss the search can judge.
    settles: Option<bool>,
    /// The class of its damage the table is asked of, `None` for a tear.
    class: Option<Class>,
    /// Whether the file's copy is damaged at the end: stale for a lost write, failing its
    /// checksum for a flip.
    stale: bool,
    must_report: bool,
    must_not_name: bool,
    /// The mark the table states for the phase the page's mapping surely ran in.
    mark: Option<Quarantine>,
    /// The marks the table states for every phase that may have mapped the page. A detection
    /// while the page was stale leaves a record that only an image or a free of the page removes.
    may: BTreeSet<Quarantine>,
    /// Whether the page's damage quarantines the database for the first open alone.
    open: bool,
    /// Whether the first open met the page, an LSN map page, out of shape and so could not
    /// prove the expected LSNs.
    unproven: bool,
    /// The row of the stated table its mark comes from.
    row: Option<Row>,
}

/// The facts of a case the rules read.
struct Facts<'a> {
    case: &'a Case<'a>,
    /// The damaged pages, the first fault's page first.
    pages: Vec<Page>,
    /// The marks the detections of the stale pages that must be reported set.
    expected: BTreeSet<Quarantine>,
    /// The marks a case may show: those of every page that lost a write, the key index's of a
    /// relation whose heap page did, whose entries can name slots the page lacks, and the
    /// database's where a walk of the mapping can meet a second damaged page before its record,
    /// or where the open quarantines the database for itself alone.
    allowed_marks: BTreeSet<Quarantine>,
    /// The pages whose failed read at the first open left it unarmed: an LSN map page or the
    /// damage list the device cannot read.
    unarmed: BTreeSet<Pid>,
    lasting: bool,
    unreadable: bool,
    /// Whether the first open met an unreadable page whose loss quarantines the database for
    /// that open only.
    open_only: bool,
    baseline: BTreeSet<Quarantine>,
}

impl Facts<'_> {
    fn primary(&self) -> Hit {
        self.case.hits[0]
    }

    /// Returns the marks the file holds beyond the sequence's deliberate damage.
    fn marks(&self) -> BTreeSet<Quarantine> {
        let all = self.case.observed.marks.clone().unwrap_or_default();
        all.difference(&self.baseline).copied().collect()
    }
}

/// Returns whether the engine's code frees pages of `kind`: the pages of relations and of the
/// large-value store, and the commit log's when it is truncated.
pub fn freed(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Heap
            | Kind::Directory
            | Kind::Index
            | Kind::VisibilityMap
            | Kind::Large
            | Kind::Engine
            | Kind::CommitLog
    )
}

/// Returns whether a page of `kind` belongs to one user relation.
fn relational(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Heap | Kind::Directory | Kind::Index | Kind::VisibilityMap
    )
}

/// A row of the stated table.
pub type Row = (Class, Held, Phase);

/// What the stated table says of one damaged page.
#[derive(Default)]
struct Judged {
    class: Option<Class>,
    mark: Option<Quarantine>,
    may: BTreeSet<Quarantine>,
    /// Whether the page's damage quarantines the database for the first open alone.
    open: bool,
    /// Whether the first open met the page unreadable while arming its read-in gate.
    unarmed: bool,
    /// Whether the first open met the page, an LSN map page, out of shape.
    unproven: bool,
    /// The row of the phase the page's mapping surely ran in.
    row: Option<Row>,
}

/// Returns what the stated table says of `fault` on the page of write-out `w`, placed `end`
/// when the consumers meet it, by the phases its record may have been mapped in. The consumers
/// read the marks in the session of the first open, which maps once each record it holds; a later
/// record is mapped once, at a call end or a commit. An earlier session may have mapped the page
/// in either phase only when the run held its record, and then while the page was placed as it
/// was when written. Returns nothing stated for a tear.
///
/// The search cannot see whether the first open's mapping of a record was complete: in a case
/// with an unreadable page, a page recorded at the open may be mapped at a later call end
/// instead, so its expected mark is the one both phases state alike.
fn judged(
    case: &Case<'_>,
    w: &WriteOut,
    fault: Fault,
    end: Holding,
    settles: Option<bool>,
) -> Judged {
    let o = case.observed;
    let met_in_open = o.met_in_open && !damages_file(fault);
    let class = match fault {
        Fault::Torn => return Judged::default(),
        _ if settles == Some(true) => Class::Settled,
        Fault::LostEarlier | Fault::LostCurrent | Fault::Recycled | Fault::Flipped => {
            Class::Damaged
        }
        Fault::UnreadableOnce | Fault::UnreadableLasting => Class::Unread,
    };
    let earlier = case.ran.recorded.contains(&w.pid);
    let incomplete = case
        .hits
        .iter()
        .any(|h| !damages_file(h.fault) && h.fault != Fault::Torn);
    let (surely, mut phases) = if met_in_open {
        (Some(Phase::Opening), vec![Phase::Opening])
    } else if o.recorded_at_open.contains(&w.pid) {
        let surely = (!incomplete || !damages_file(fault)).then_some(Phase::Opening);
        (surely, vec![Phase::Opening])
    } else {
        (Some(Phase::Running), vec![Phase::Running])
    };
    if earlier || surely.is_none() {
        phases = PHASES.to_vec();
    }
    let mark = |p| stated::durable(class, end, p);
    let agreed = PHASES
        .iter()
        .map(|p| mark(*p))
        .reduce(|a, b| if a == b { a } else { None });
    let written = case
        .plan
        .placed_written(*w)
        .map(|placed| placed.0)
        .filter(|_| earlier);
    let held = [Some(end), written];
    let unknown = is_lost(fault) && case.plan.ids.status.contains(&w.pid) && settles.is_none();
    Judged {
        class: Some(class),
        mark: surely.map_or(agreed.flatten(), mark).filter(|_| !unknown),
        may: phases
            .iter()
            .flat_map(|p| {
                held.iter()
                    .flatten()
                    .filter_map(|h| stated::durable(class, *h, *p))
            })
            .collect(),
        open: met_in_open && stated::stated(class, end.0, Phase::Opening) == Stated::Open,
        unarmed: met_in_open && class == Class::Unread && end.0 == Held::LsnMap,
        unproven: class == Class::Damaged
            && end.0 == Held::LsnMap
            && surely == Some(Phase::Opening)
            && !unknown,
        row: surely.map(|p| (class, end.0, p)).filter(|_| !unknown),
    }
}

/// Returns whether the commit log settles the status page `w` writes when `fault` lost the write,
/// at the first open after the loss: the page is stale there, no later write-out of it having
/// landed before it, and the copy the file holds, the one its last write-out before `w` wrote,
/// holds no id in progress below the first id not reserved and from the truncation bound the
/// plan's run had then, or the retained log holds a change or image of the page continuing from
/// the copy up to `w`. Returns `false` for a page that is no status page that lost a write, and
/// `None` when the search cannot judge that open: past a recycle of the log, beside a second
/// fault, a lost write to the log or a crash the fault caused, for a write-out during an open, or
/// when the log the file retains begins past the copy and holds no such record.
fn settles(case: &Case<'_>, w: &WriteOut, fault: Fault) -> Option<bool> {
    let plan = case.plan;
    if !is_lost(fault) || !plan.ids.status.contains(&w.pid) {
        return Some(false);
    }
    if fault == Fault::Recycled
        || case.hits.len() > 1
        || case.lost_log.is_some()
        || case.ran.cut.is_some()
    {
        return None;
    }
    let last = plan.clog.len().checked_sub(1)?;
    let before = plan.opens.iter().rev().find(|open| open.0 <= w.op);
    if before.is_some_and(|open| open.2 == w.during) {
        return None;
    }
    let (outs, op) = plan
        .opens
        .iter()
        .find(|open| open.0 > w.op)
        .map_or((u64::MAX, last), |open| (open.0, open.1));
    let writes = &plan.writes;
    if writes
        .iter()
        .any(|x| x.pid == w.pid && x.op > w.op && x.op < outs)
    {
        return Some(false);
    }
    let Some(at) = writes.iter().rposition(|x| x.pid == w.pid && x.op < w.op) else {
        return Some(false);
    };
    let copy = writes[at].lsn;
    let image = plan.images.get(&at)?;
    let (reserved, bound) = *plan.clog.get(op)?;
    if !ledger::in_progress(image, reserved, bound) {
        return Some(true);
    }
    ledger::continues(&case.ran.files, w.pid, copy, w.lsn)
}

pub fn is_lost(fault: Fault) -> bool {
    matches!(
        fault,
        Fault::LostEarlier | Fault::LostCurrent | Fault::Recycled
    )
}

/// Returns whether `fault` leaves on the file a copy of its page that the pager refuses as
/// damaged.
fn damages_file(fault: Fault) -> bool {
    is_lost(fault) || fault == Fault::Flipped
}

/// Returns the write-out of the incarnation of `w`'s page the consumers meet: the clean run's
/// last, when the sequence frees `w`'s incarnation and the faulted run last wrote the page as the
/// same kind; else `w`.
fn incarnation_met(case: &Case<'_>, w: WriteOut) -> WriteOut {
    let reused = case.plan.incarnation_at_end(w);
    let writes = case.ran.trace.writes.iter().rev();
    let faulted = writes.copied().find(|x| x.pid == w.pid);
    if faulted.is_some_and(|x| x.kind == reused.kind) {
        reused
    } else {
        w
    }
}

/// Returns whether the faulted run wrote out the pages the plan's run did, in its order.
fn followed(case: &Case<'_>) -> bool {
    case.ran.trace.writes == case.plan.writes
}

fn facts<'a>(case: &'a Case<'a>) -> Facts<'a> {
    let plan = case.plan;
    let ids = &plan.ids;
    let mut pages = Vec::new();
    let mut open_only = false;
    let mut unarmed = BTreeSet::new();
    let end = case.ran.cut.unwrap_or(usize::MAX);
    let follows = followed(case);
    for hit in &case.hits {
        let written = match hit.fault {
            Fault::UnreadableLasting => incarnation_met(case, hit.written),
            _ => hit.written,
        };
        let mut batch = vec![written];
        if is_lost(hit.fault) {
            let writes = &plan.writes;
            batch.extend(
                writes
                    .iter()
                    .filter(|x| x.op == hit.written.op && x.pid != hit.written.pid),
            );
        }
        for w in batch {
            let Some(kind) = world::class(w.kind, w.owner, ids) else {
                continue;
            };
            let written = plan.placed_written(w);
            let at_end = plan.placed_at(w.pid, end);
            let placed = match (follows, at_end) {
                (true, Some(at_end)) if hit.fault != Fault::UnreadableOnce => at_end,
                _ => written.unwrap_or(world::FREE),
            };
            let rel = written.unwrap_or(placed).1;
            let copy = world::page(&case.observed.files, w.pid);
            let found = page::lsn(&copy).0;
            let stale = match hit.fault {
                Fault::Flipped => !case::sealed(w.pid, &copy),
                fault => is_lost(fault) && found < w.lsn,
            };
            let alive = at_end.is_none_or(|p| p.0.0 != Held::Free);
            let must_report = matches!(
                hit.fault,
                Fault::LostCurrent | Fault::Recycled | Fault::Flipped
            ) && stale
                && alive
                && !unlogged_before_gap(case, &w, found);
            let must_not_name = case.ran.damaged != Some(w.pid)
                && match hit.fault {
                    Fault::Torn | Fault::UnreadableOnce => true,
                    Fault::LostEarlier => !stale,
                    _ => false,
                };
            let settles = settles(case, &w, hit.fault);
            let judged = judged(case, &w, hit.fault, placed.0, settles);
            open_only |= judged.open || judged.unproven;
            if judged.unarmed {
                unarmed.insert(w.pid);
            }
            pages.push(Page {
                pid: w.pid,
                kind,
                fault: hit.fault,
                rel,
                at_end: at_end
                    .filter(|p| follows && p.0.0 != Held::Free)
                    .map(|p| p.0.0),
                settles,
                class: judged.class,
                stale,
                must_report,
                must_not_name,
                mark: judged.mark,
                may: judged.may,
                open: judged.open,
                unproven: judged.unproven,
                row: judged.row,
            });
        }
    }
    let unreadable_pids: BTreeSet<Pid> = case
        .hits
        .iter()
        .filter(|h| h.fault == Fault::UnreadableLasting)
        .map(|h| h.written.pid)
        .collect();
    for p in pages
        .iter_mut()
        .filter(|p| unreadable_pids.contains(&p.pid))
    {
        p.must_not_name = false;
    }
    let expected: BTreeSet<Quarantine> = pages
        .iter()
        .filter(|p| p.must_report)
        .filter_map(|p| p.mark)
        .collect();
    unarmed.extend(
        case.hits
            .iter()
            .filter(|h| h.fault == Fault::UnreadableLasting && h.kind == Kind::DamageList)
            .map(|h| h.written.pid),
    );
    let lasting = case.lost_log.as_ref().is_some_and(|l| !l.tail || l.named)
        || case
            .hits
            .iter()
            .any(|h| h.fault == Fault::UnreadableLasting && h.kind != Kind::DamageList);
    let unreadable = case
        .hits
        .iter()
        .any(|h| matches!(h.fault, Fault::UnreadableOnce | Fault::UnreadableLasting));
    let named: BTreeSet<Pid> = case
        .observed
        .verify
        .iter()
        .flatten()
        .flatten()
        .flat_map(|(_, at)| pages.iter().map(|p| p.pid).filter(|pid| names(at, *pid)))
        .collect();
    let mut allowed_marks = lost_page_keeps_its_mark(&pages, &named);
    allowed_marks.extend(unused_slot_marks_its_index(&pages, ids));
    if open_only {
        allowed_marks.insert(Quarantine::Database);
    }
    if let Some(lost) = &case.lost_log {
        allowed_marks.extend(past_lost_log(case, lost));
    }
    let mut baseline = BTreeSet::new();
    if plan.ops.contains(&Op::Damage)
        && let Some((h, _)) = ids.marks.get(&Rel::Scraps)
    {
        baseline.insert(*h);
    }
    Facts {
        case,
        pages,
        expected,
        allowed_marks,
        unarmed,
        lasting,
        unreadable,
        open_only,
        baseline,
    }
}

/// Returns the mark of every page that lost a write, whatever its state at the end: a detection
/// while it was stale leaves a record that only an image or a free of the page removes. The
/// database's mark comes only from such a page that must be reported or that verify names.
fn lost_page_keeps_its_mark(pages: &[Page], named: &BTreeSet<Pid>) -> BTreeSet<Quarantine> {
    pages
        .iter()
        .flat_map(|p| {
            let database = p.must_report || named.contains(&p.pid);
            p.may
                .iter()
                .filter(move |m| **m != Quarantine::Database || database)
        })
        .copied()
        .collect()
}

/// Returns the key index mark of each relation whose heap page lost a write: an index entry
/// naming an unused slot marks its index at the call that finds it.
fn unused_slot_marks_its_index(pages: &[Page], ids: &world::Ids) -> Vec<Quarantine> {
    pages
        .iter()
        .filter(|p| !p.may.is_empty() && p.kind == Kind::Heap)
        .filter_map(|p| p.rel.and_then(|r| ids.marks.get(&r)))
        .map(|(_, index)| *index)
        .collect()
}

/// Returns the write-outs of the process that lost a write to the log, submitted after it, at or
/// past its LSN.
fn past_lost<'a>(case: &'a Case<'_>, lost: &'a LostLog) -> impl Iterator<Item = &'a WriteOut> {
    case.ran
        .trace
        .writes
        .iter()
        .filter(|w| lost.after.contains(&w.op) && w.lsn >= lost.lsn)
}

/// Returns the marks a lost write to the log allows: the object holding each page its process
/// wrote after it at or past its LSN, the database for a page no heap or index holds, and the
/// database for a lost batch before the log's last.
fn past_lost_log(case: &Case<'_>, lost: &LostLog) -> BTreeSet<Quarantine> {
    let mut out = BTreeSet::new();
    if !lost.tail {
        out.insert(Quarantine::Database);
    }
    for w in past_lost(case, lost) {
        let object = case.plan.placed_written(*w).and_then(|placed| placed.0.1);
        out.insert(object.unwrap_or(Quarantine::Database));
    }
    out
}

/// Returns whether a lost write to the log waives the report of the stale page of write-out `w`,
/// whose copy on file has LSN `found`: its record lies at or past the lost write, and the retained
/// log holds no change or image of the page past `found`.
fn unlogged_before_gap(case: &Case<'_>, w: &WriteOut, found: u64) -> bool {
    let Some(lost) = &case.lost_log else {
        return false;
    };
    w.lsn >= lost.lsn
        && ledger::last_logged(&case.ran.files, w.pid).is_none_or(|last| last <= found)
}

/// Returns the pages a lost last batch of the log changed that reached the file and that verify
/// reports neither as unlogged nor as stale.
fn unlogged_unnamed(case: &Case<'_>, rows: &[Finding]) -> Vec<Pid> {
    let Some(lost) = case.lost_log.as_ref().filter(|l| l.tail) else {
        return Vec::new();
    };
    past_lost(case, lost)
        .filter(|w| page::lsn(&world::page(&case.observed.files, w.pid)).0 == w.lsn)
        .map(|w| w.pid)
        .filter(|pid| !rows.iter().any(|row| unlogged(row, *pid)))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Returns whether verify's `row` reports page `pid` as unlogged or stale.
fn unlogged((kind, at): &Finding, pid: Pid) -> bool {
    (kind == "StalePage" && names(at, pid))
        || at.contains(&format!("Unlogged {{ pid: Pid({}),", pid.0))
}

/// Returns whether `all` holds an allowed mark on the database, which quarantines every heap and
/// index.
fn database_mark_covers_relations(f: &Facts<'_>, all: &BTreeSet<Quarantine>) -> bool {
    all.contains(&Quarantine::Database) && f.allowed_marks.contains(&Quarantine::Database)
}

/// Returns whether a page the device cannot read for good is one a mapping walk reads, which
/// then leaves the mapping incomplete and marks nothing when it found no holder.
fn unreadable_page_leaves_mapping_incomplete(f: &Facts<'_>) -> bool {
    f.pages.iter().any(|p| {
        p.fault == Fault::UnreadableLasting
            && matches!(
                p.kind,
                Kind::Heap
                    | Kind::Directory
                    | Kind::Index
                    | Kind::VisibilityMap
                    | Kind::Large
                    | Kind::Engine
            )
    })
}

/// Returns whether `e`, the failure of the consumers' open, is the I/O failure of a damage list
/// page unreadable for good, which leaves the open unarmed, and the open was given the migration
/// whose open failed in the run: an unarmed open given a migration fails naming the failure.
fn unarmed_open_given_a_migration(f: &Facts<'_>, e: &str) -> bool {
    let case = f.case;
    let list_unread = case.hits.iter().any(|h| {
        h.fault == Fault::UnreadableLasting
            && h.kind == Kind::DamageList
            && e.contains(&format!(
                "Unarmed(Page(PageError {{ pid: Pid({}), damage: Unreadable }}))",
                h.written.pid.0
            ))
    });
    let end = case.ran.cut.map_or(case.plan.ops.len(), |c| c + 1);
    let last_migration = (0..end)
        .rev()
        .find(|i| case.plan.ops[*i] == Op::Migrate && !case.ran.refused_under_mark.contains(i));
    let unapplied = last_migration
        .and_then(|i| case.ran.failed.get(&i))
        .is_some_and(|failed| failed.starts_with("open: "));
    list_unread && unapplied
}

/// Returns why the database is marked, by what the damaged pages are.
fn database_cause(f: &Facts<'_>) -> &'static str {
    if f.pages.iter().all(|p| p.class == Some(Class::Settled)) {
        "for a settled status page"
    } else if f
        .pages
        .iter()
        .any(|p| p.rel.is_some() && damages_file(p.fault))
    {
        "the faults map to their relation"
    } else if f.pages.iter().all(|p| !damages_file(p.fault)) {
        "for an unreadable page"
    } else {
        "the faults do not map to"
    }
}

pub fn judge(case: &Case<'_>) -> Verdicts {
    let f = facts(case);
    let o = case.observed;
    let mut out = Vec::new();
    let mut v = |consumer: Consumer, what: String, detail: String| {
        out.push(Violation {
            consumer,
            what,
            detail,
        });
    };
    if let Some(p) = &o.panic {
        v(Consumer::Settle, "the engine panics".into(), p.clone());
    }
    let (placing, walked, settled) = judge_placing(&f);
    for (what, detail) in placing {
        v(Consumer::Settle, what, detail);
    }
    if let Some(Err(e)) = &o.open
        && !f.lasting
        && !unarmed_open_given_a_migration(&f, e)
    {
        v(Consumer::Settle, "the open fails".into(), e.clone());
    }
    catalog(&f, &mut v);
    for (rel, read) in &o.reads {
        if let Some((what, detail)) = judge_read(&f, *rel, read, None, "") {
            v(Consumer::Read, what, detail);
        }
    }
    for (rel, l) in &o.lookups {
        if let Some((what, detail)) = judge_read(&f, *rel, &l.read, Some(&l.keys), "by key: ") {
            v(Consumer::Read, what, detail);
        }
    }
    if let Some(found) = &o.verify {
        for (what, detail) in judge_findings(&f, found) {
            v(Consumer::Verify, what, detail);
        }
        if let Ok(rows) = found {
            let unnamed = unlogged_unnamed(case, rows);
            if !unnamed.is_empty() {
                v(
                    Consumer::Verify,
                    "verify misses a page past the log's end".into(),
                    format!("pages {unnamed:?}: {rows:?}"),
                );
            }
        }
    }
    if o.marks.is_some() || o.reopened.is_some() {
        for (what, detail) in judge_marks(&f) {
            v(Consumer::Verify, what, detail);
        }
    }
    if let Some(b) = &o.backup {
        for (what, detail) in judge_backup(&f, b) {
            v(Consumer::Backup, what, detail);
        }
    }
    if let Some(d) = &o.dump {
        for (what, detail) in judge_dump(&f, d) {
            v(Consumer::Dump, what, detail);
        }
    }
    let mut held = (0, 0);
    if let Some((snapshots, ended)) = case.evidence {
        let records: BTreeSet<&Record> = snapshots.iter().flat_map(|s| &s.records).collect();
        let marks: BTreeSet<&Quarantine> =
            snapshots.iter().flat_map(|s| &s.marks).flatten().collect();
        held = (records.len(), marks.len());
        if let Some(detail) = record_lost(snapshots, ended) {
            v(Consumer::Settle, RECORD_LOST.into(), detail);
        }
        if let Some(detail) = mark_lost(snapshots) {
            v(Consumer::Settle, MARK_LOST.into(), detail);
        }
        for (what, detail) in durable_marks_unmet(&f, snapshots) {
            v(Consumer::Read, what, detail);
        }
    }
    Verdicts {
        violations: out,
        rows: asked(&f),
        reached: reached(&f),
        met: met(&f, &f.pages[0]),
        named: named(&f),
        ending: ending(&f),
        evidence: held,
        walked,
        settled,
    }
}

fn catalog(f: &Facts<'_>, v: &mut impl FnMut(Consumer, String, String)) {
    if let Some(i) = f.case.ran.under_mark.first() {
        v(
            Consumer::Catalog,
            "a migration under a whole-database mark returns Ok".into(),
            format!("operation {i}"),
        );
    }
    if let Some(i) = f.case.ran.applied_under_mark.first() {
        v(
            Consumer::Catalog,
            "applies a migration under a whole-database mark".into(),
            format!("operation {i}"),
        );
    }
    let tags = f.case.plan.ids.marks.get(&Rel::Tags).map(|(heap, _)| *heap);
    for (i, (version, marks)) in &f.case.ran.refused_by_relation {
        let reads = *version == world::LAST && tags.is_some_and(|heap| marks.contains(&heap));
        if !reads {
            v(
                Consumer::Catalog,
                "a migration is refused by the mark of a relation it does not read".into(),
                format!("operation {i}, version {version}, marks {marks:?}"),
            );
        }
    }
    let Some(relations) = &f.case.observed.relations else {
        return;
    };
    let tags = relations.contains(Rel::Tags.name());
    let version = f.case.ran.acked.version;
    let started = f.case.started;
    let unread = relations.is_empty()
        || f.case.observed.verify.as_ref().is_some_and(|found| {
            found.as_ref().is_ok_and(|rows| {
                rows.iter()
                    .any(|(c, at)| c == "Structure" && at.contains("catalog"))
            })
        });
    let applied = (version..=started.max(version)).any(|v| world::present(Rel::Tags, v) == tags);
    if !applied && !unread {
        v(
            Consumer::Catalog,
            "an acknowledged migration reads as not applied".into(),
            format!("version {version}, relations {relations:?}"),
        );
    }
}

/// Returns the marks a refusal naming `what` can be for: the database's, or the heap's and key
/// index's of the relation it names.
fn refused_marks(f: &Facts<'_>, what: &str) -> Vec<Quarantine> {
    if what == "the database" {
        return vec![Quarantine::Database];
    }
    let ids = &f.case.plan.ids;
    ids.marks
        .iter()
        .filter(|(r, _)| {
            what.strip_prefix(&r.qualified()).is_some_and(|rest| {
                rest.is_empty() || rest.starts_with('.') || rest.starts_with(' ')
            })
        })
        .flat_map(|(_, (h, i))| [*h, *i])
        .collect()
}

/// Returns the violation a read of `rel` shows, `keys` the keys a read by key asked for and
/// `prefix` naming the read.
fn judge_read(
    f: &Facts<'_>,
    rel: Rel,
    read: &Read,
    keys: Option<&[i64]>,
    prefix: &str,
) -> Option<(String, String)> {
    let name = rel.name();
    let empty = BTreeMap::new();
    let allowed: Vec<BTreeMap<i64, u32>> = f
        .case
        .allowed
        .iter()
        .map(|s| {
            s.get(&rel)
                .unwrap_or(&empty)
                .iter()
                .filter(|(id, _)| keys.is_none_or(|k| k.contains(id)))
                .map(|(id, v)| (*id, *v))
                .collect()
        })
        .collect();
    match read {
        Read::Rows(rows) if allowed.contains(rows) => None,
        Read::Rows(rows) => Some((
            format!("{prefix}wrong rows from {name}"),
            format!("got {rows:?}, acknowledged {:?}", allowed.last()),
        )),
        Read::Garbage(row) => Some((
            format!("{prefix}a value no write gave from {name}"),
            row.clone(),
        )),
        Read::Refused(what) => {
            let all = f.case.observed.marks.clone().unwrap_or_default();
            let marked = refused_marks(f, what).into_iter().any(|m| {
                all.contains(&m) && (f.allowed_marks.contains(&m) || f.baseline.contains(&m))
            });
            (!marked).then(|| {
                let cause = if what != "the database" {
                    if refused_marks(f, what).iter().any(|m| all.contains(m)) {
                        "refused a relation with a mark the faults do not map to".to_string()
                    } else {
                        "refused a relation with no mark set".to_string()
                    }
                } else if all.contains(&Quarantine::Database) {
                    format!(
                        "refused the database with a Database mark {}",
                        database_cause(f)
                    )
                } else {
                    "refused the database with no Database mark".to_string()
                };
                (
                    format!("{prefix}{cause}"),
                    format!("{name}: {what}; marks {all:?}"),
                )
            })
        }
        Read::Failed(e) => {
            let explained = f.pages.iter().any(|p| {
                p.fault == Fault::UnreadableLasting
                    && (names_pid(e, p.pid) || !relational(p.kind) || p.rel == Some(rel))
            });
            (!explained).then(|| (format!("{prefix}{name} fails"), e.clone()))
        }
    }
}

/// Returns the failure a call returns on an open a read of `pid` left unarmed.
fn unreadable(pid: Pid) -> String {
    let detail = format!("page {} is unreadable", pid.0);
    format!(
        "{:?}",
        adtb::Error::Fatal {
            reason: adtb::FatalReason::Storage { detail },
        }
    )
}

/// Returns whether `at`, the place of a finding, names page `pid`.
fn names(at: &str, pid: Pid) -> bool {
    let page = format!("\"page {}", pid.0);
    at.strip_prefix(&page)
        .is_some_and(|rest| rest.starts_with(':') || rest.starts_with(' '))
}

fn judge_findings(f: &Facts<'_>, found: &Result<Vec<Finding>, String>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let rows = match found {
        Ok(rows) => rows,
        Err(e) => {
            if !f.lasting {
                out.push(("verify fails".into(), e.clone()));
            }
            return out;
        }
    };
    for p in &f.pages {
        let named = rows.iter().any(|(_, at)| names(at, p.pid));
        if p.must_report && !named {
            out.push((
                "verify misses a stale page".into(),
                format!("page {}: {rows:?}", p.pid.0),
            ));
        }
        if p.must_not_name && named {
            out.push((
                "verify names an intact page".into(),
                format!("page {}: {rows:?}", p.pid.0),
            ));
        }
    }
    let extra = f.marks();
    if !extra.is_empty() && rows.is_empty() {
        out.push((
            "verify reports nothing of a mark".into(),
            format!("{extra:?}"),
        ));
    }
    out
}

/// Returns the violations of the marks the file holds. Under unreadable pages alone: no mark
/// beyond those the stated table gives the pages, but the open's own quarantine of the database;
/// a read refused by that quarantine, or failing as a storage failure naming a page that left the
/// open unarmed; and the next open with the page readable holding neither while it serves every
/// row. Else every mark the stale pages map to, unless the
/// database's mark, which covers every object, is set and allowed, or an unreadable page leaves
/// the mapping incomplete; and no mark beyond those the case may show.
fn judge_marks(f: &Facts<'_>) -> Vec<(String, String)> {
    let marks = f.marks();
    let mut out = Vec::new();
    if f.unreadable && !f.case.hits.iter().any(|h| damages_file(h.fault)) {
        if f.open_only || !f.unarmed.is_empty() {
            let o = f.case.observed;
            let unarmed: BTreeSet<String> = f.unarmed.iter().map(|p| unreadable(*p)).collect();
            let refused = o
                .reads
                .values()
                .chain(o.lookups.values().map(|l| &l.read))
                .all(|r| match r {
                    Read::Failed(e) => unarmed.contains(e),
                    Read::Refused(what) => unarmed.is_empty() && what == "the database",
                    Read::Rows(_) | Read::Garbage(_) => false,
                });
            let held = if f.open_only {
                BTreeSet::from([Quarantine::Database])
            } else {
                BTreeSet::new()
            };
            if marks != held || !refused {
                out.push((
                    "an open that cannot read the page serves past it".into(),
                    format!("marks {marks:?}; reads {:?}", o.reads),
                ));
            }
            if let Some(again) = &o.reopened {
                let left: BTreeSet<_> = again.marks.difference(&f.baseline).copied().collect();
                let served = again.reads.iter().all(|(rel, read)| {
                    matches!(read, Read::Rows(_)) && judge_read(f, *rel, read, None, "").is_none()
                });
                if again.open.is_err() || !left.is_empty() || !served {
                    out.push((
                        "the open's quarantine outlives the open".into(),
                        format!("{:?}; marks {left:?}; reads {:?}", again.open, again.reads),
                    ));
                }
            }
        } else {
            let stated: BTreeSet<Quarantine> =
                f.pages.iter().flat_map(|p| &p.may).copied().collect();
            let marks: BTreeSet<Quarantine> = marks.difference(&stated).copied().collect();
            if !marks.is_empty() {
                let shape = if marks.contains(&Quarantine::Database) {
                    "an unreadable page marks the database"
                } else {
                    "an unreadable page marks a relation"
                };
                out.push((shape.into(), format!("{marks:?}")));
            }
            if let Some(again) = &f.case.observed.reopened {
                let left: BTreeSet<_> = again.marks.difference(&f.baseline).copied().collect();
                if !left.is_empty() {
                    out.push((
                        "an unreadable page leaves a mark the next open holds".into(),
                        format!("{left:?}"),
                    ));
                }
            }
        }
        return out;
    }
    if f.case.observed.marks.is_none() {
        return out;
    }
    let all = f.case.observed.marks.clone().unwrap_or_default();
    let missing: BTreeSet<_> = f.expected.difference(&all).collect();
    if !missing.is_empty()
        && !database_mark_covers_relations(f, &all)
        && !unreadable_page_leaves_mapping_incomplete(f)
    {
        let shape = if all.contains(&Quarantine::Database) {
            "the database is marked in place of the mark a stale page maps to"
        } else {
            "the mark a stale page maps to is missing"
        };
        out.push((
            shape.into(),
            format!("missing {missing:?}, marks {marks:?}"),
        ));
    }
    let over: BTreeSet<_> = marks.difference(&f.allowed_marks).collect();
    if !over.is_empty() {
        let shape = if !over.contains(&Quarantine::Database) {
            "a mark the faults do not map to"
        } else if f.pages.iter().any(|p| p.rel.is_some()) {
            "the database is marked for pages that map to their relation"
        } else if f.pages.iter().all(|p| p.class == Some(Class::Settled)) {
            "the database is marked for a settled status page"
        } else {
            "a mark the faults do not map to"
        };
        let pages: Vec<_> = f.pages.iter().map(|p| (p.pid.0, p.stale)).collect();
        out.push((
            shape.into(),
            format!(
                "over {over:?}, expected {:?}; pages and staleness {pages:?}; engine settled {:?}",
                f.expected, f.case.observed.settled
            ),
        ));
    }
    out
}

fn judge_copy(f: &Facts<'_>, copy: &Copy, prefix: &str) -> Vec<(String, String)> {
    copy.reads
        .iter()
        .filter_map(|(rel, read)| match read {
            Read::Rows(_) | Read::Garbage(_) => judge_read(f, *rel, read, None, prefix),
            Read::Refused(_) | Read::Failed(_) => None,
        })
        .collect()
}

fn judge_backup(f: &Facts<'_>, b: &Backup) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let marks = f.case.observed.marks.clone().unwrap_or_default();
    if let Err(e) = &b.made
        && !f.lasting
        && !marks.contains(&Quarantine::Database)
        && !matches!(e, Read::Failed(e) if f.unarmed.iter().any(|p| *e == unreadable(*p)))
    {
        out.push(("the backup fails".into(), format!("{e:?}")));
    }
    if let Some(Ok(copy)) = &b.copy {
        out.extend(judge_copy(f, copy, "the backup's copy: "));
        let proven = matches!(b.proof, Some(Ok(Proof::Proven)));
        let clean = copy.verify.as_ref().is_ok_and(Vec::is_empty);
        let intact = judge_copy(f, copy, "").is_empty();
        if proven && !(clean && intact) {
            out.push((
                "the proof passes over a damaged copy".into(),
                format!("{:?}", copy.verify),
            ));
        }
        if proven && f.pages.iter().any(|p| p.must_report) {
            out.push(("the proof passes over a stale page".into(), String::new()));
        }
    }
    out
}

fn judge_dump(f: &Facts<'_>, d: &Dumped) -> Vec<(String, String)> {
    let marks = f.case.observed.marks.clone().unwrap_or_default();
    let mut out = Vec::new();
    if let Err(e) = &d.made
        && !f.lasting
        && marks.is_empty()
    {
        out.push(("the dump fails".into(), format!("{e:?}")));
    }
    if let Some(Ok(copy)) = &d.copy {
        out.extend(judge_copy(f, copy, "the dump's copy: "));
    }
    out
}

/// Returns the consumers that ran on the first fault's file. A read meets an index page only
/// through a read by key that the plan routes through that relation's key index.
fn reached(f: &Facts<'_>) -> BTreeSet<Consumer> {
    let o = f.case.observed;
    let p = f.primary();
    let ids = &f.case.plan.ids;
    let through = |l: &Lookups| l.probes;
    let read = match p.kind {
        Kind::Index => f.pages[0].rel.is_some_and(|rel| {
            ids.key_indexes.get(&rel) == Some(&p.written.owner)
                && o.lookups.get(&rel).is_some_and(through)
        }),
        _ => !o.reads.is_empty(),
    };
    [
        (Consumer::Settle, o.open.is_some()),
        (Consumer::Catalog, o.relations.is_some()),
        (Consumer::Read, read),
        (Consumer::Verify, o.verify.is_some()),
        (Consumer::Backup, o.backup.is_some()),
        (Consumer::Dump, o.dump.is_some()),
    ]
    .into_iter()
    .filter_map(|(c, ran)| ran.then_some(c))
    .collect()
}

/// Returns the consumers whose answer carries page `p`: the open recorded, settled or could not
/// read it, or failed naming it; a read was refused by a mark the page maps to or failed naming
/// it, or an operation of the faulted run failed naming it; verify named it; the backup, its proof
/// or its copy, or the dump or its copy, named it or was refused by a mark it maps to.
fn met(f: &Facts<'_>, p: &Page) -> BTreeSet<Consumer> {
    let o = f.case.observed;
    let pid = p.pid;
    let by_page = |r: &Read| match r {
        Read::Refused(what) => refused_marks(f, what)
            .iter()
            .any(|m| p.may.contains(m) || p.open && *m == Quarantine::Database),
        Read::Failed(e) => names_pid(e, pid),
        Read::Rows(_) | Read::Garbage(_) => false,
    };
    let read = o
        .reads
        .values()
        .chain(o.lookups.values().map(|l| &l.read))
        .any(by_page)
        || f.case.ran.failed.values().any(|e| names_pid(e, pid));
    let copy = |c: &Option<Result<Copy, String>>| match c {
        Some(Ok(copy)) => {
            copy.reads.values().any(by_page)
                || copy
                    .verify
                    .as_ref()
                    .is_ok_and(|rows| rows.iter().any(|(_, at)| names(at, pid)))
        }
        Some(Err(e)) => names_pid(e, pid),
        None => false,
    };
    let lasting = f
        .case
        .hits
        .iter()
        .any(|h| h.fault == Fault::UnreadableLasting && h.written.pid == pid);
    let open = (lasting && o.met_in_open)
        || o.recorded_at_open.contains(&pid)
        || o.settled.contains(&pid)
        || matches!(&o.open, Some(Err(e)) if names_pid(e, pid));
    let verify = o.verify.as_ref().is_some_and(|r| {
        r.as_ref()
            .is_ok_and(|rows| rows.iter().any(|(_, at)| names(at, pid)))
    });
    let backup = o.backup.as_ref().is_some_and(|b| {
        b.made.as_ref().is_err_and(&by_page)
            || match &b.proof {
                Some(Ok(Proof::Failed(found))) => found.iter().any(|t| names_pid(t, pid)),
                Some(Err(e)) => names_pid(e, pid),
                _ => false,
            }
            || copy(&b.copy)
    });
    let dump = o
        .dump
        .as_ref()
        .is_some_and(|d| d.made.as_ref().is_err_and(&by_page) || copy(&d.copy));
    [
        (Consumer::Settle, open),
        (Consumer::Read, read),
        (Consumer::Verify, verify),
        (Consumer::Backup, backup),
        (Consumer::Dump, dump),
    ]
    .into_iter()
    .filter_map(|(c, met)| met.then_some(c))
    .collect()
}

/// Returns whether the text of an error or a finding names page `pid`, as `Pid(N)` or `page N`.
fn names_pid(text: &str, pid: Pid) -> bool {
    let bounded = |needle: String| {
        text.match_indices(&needle)
            .any(|(at, _)| !text[at + needle.len()..].starts_with(|c: char| c.is_ascii_digit()))
    };
    bounded(format!("Pid({})", pid.0)) || bounded(format!("page {}", pid.0))
}

/// Returns the rows of the stated table the case asked: for each page a consumer met, the row
/// of the phase its mapping surely ran in, when the judge checked what the row states. A row
/// stating a mark is asked when the judge required that mark held; a row stating nothing when it
/// checked the marks held nothing beyond what the faults allow; a row holding the database for the
/// open when it checked that the open held it, and for an LSN map page out of shape at the open
/// also the row of the open that could not prove the expected LSNs.
fn asked(f: &Facts<'_>) -> BTreeSet<Row> {
    let Some(all) = &f.case.observed.marks else {
        return BTreeSet::new();
    };
    let excused = unreadable_page_leaves_mapping_incomplete(f);
    let covered = database_mark_covers_relations(f, all);
    let unproven = (Class::Unarmed, Held::LsnMap, Phase::Opening);
    f.pages
        .iter()
        .filter(|p| !met(f, p).is_empty())
        .flat_map(|p| {
            let checked = |row: Row| match stated::stated(row.0, row.1, row.2) {
                Stated::Own | Stated::Database => {
                    p.must_report
                        && !excused
                        && p.mark
                            .is_some_and(|m| m == Quarantine::Database || !covered)
                }
                Stated::Nothing => true,
                Stated::Open => (p.open || p.unproven) && f.open_only,
            };
            let unproven = p.unproven.then_some(unproven);
            p.row
                .into_iter()
                .chain(unproven)
                .filter(move |r| checked(*r))
        })
        .collect()
}

/// Returns the violations of where the plan placed each damaged page against the engine: the
/// holders the mapping's walk finds once the consumers ran, for a page placed at the end, and
/// whether the commit log settled a status page, for one the search judges settled or not. Returns
/// also whether it compared a walk and whether it compared a settle.
fn judge_placing(f: &Facts<'_>) -> (Vec<(String, String)>, bool, bool) {
    let o = f.case.observed;
    let mut out = Vec::new();
    let (mut walked, mut settled) = (false, false);
    let read = o.relations.as_ref().is_some_and(|r| !r.is_empty());
    for p in &f.pages {
        if let (Some(Some(holders)), Some(at_end), true) = (o.walked.get(&p.pid), p.at_end, read) {
            walked = true;
            let held: BTreeSet<Held> = holders.iter().map(|h| stated::holding(*h).0).collect();
            if !held.contains(&at_end) {
                out.push((
                    "the mapping's walk holds a page elsewhere than the plan placed it".into(),
                    format!("page {}: placed {at_end:?}, walked {holders:?}", p.pid.0),
                ));
            }
        }
        if let (Some(settles), Some(Ok(()))) = (p.settles, &o.open)
            && is_lost(p.fault)
            && f.case.plan.ids.status.contains(&p.pid)
        {
            settled = true;
            if settles != o.settled.contains(&p.pid) {
                let what = if settles {
                    "the commit log leaves unsettled a status page the plan and its log settle"
                } else {
                    "the commit log settles a status page the plan and its log do not settle"
                };
                out.push((what.into(), format!("page {}", p.pid.0)));
            }
        }
    }
    (out, walked, settled)
}

fn named(f: &Facts<'_>) -> Vec<String> {
    let pid = f.pages[0].pid;
    match &f.case.observed.verify {
        Some(Ok(rows)) => rows
            .iter()
            .filter(|(_, at)| names(at, pid))
            .map(|(_, at)| {
                let damage = at.split_once(": ").map_or("", |(_, d)| d);
                damage
                    .split([' ', '"'])
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn ending(f: &Facts<'_>) -> Ending {
    let o = f.case.observed;
    let failed = matches!(o.open, Some(Err(_)))
        || o.reads.values().any(|r| matches!(r, Read::Failed(_)))
        || o.verify.as_ref().is_some_and(Result::is_err);
    let marks = f.marks();
    let all = o.marks.clone().unwrap_or_default();
    let named = o
        .verify
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .is_some_and(|rows| {
            f.pages
                .iter()
                .any(|p| rows.iter().any(|(_, at)| names(at, p.pid)))
        });
    let stale = f.pages.iter().any(|p| p.stale);
    if failed {
        Ending::StorageFailure
    } else if (named || stale) && !f.expected.iter().all(|m| all.contains(m)) {
        Ending::Unmarked
    } else if named || stale || !marks.is_empty() {
        Ending::Reported
    } else {
        Ending::Intact
    }
}

const RECORD_LOST: &str = "a damage record the log held reaches neither the damage list nor a \
                               durable mark";
const MARK_LOST: &str = "a durable mark is gone before its release";

/// Returns where the first damage record an observation of `snapshots` held is, at that or a
/// later one, in no damage list, not in the log past the redo LSN, ended by no free or image in the
/// log then or in `ended`, and not covered by a durable mark of the database while the list page
/// holds no list; `None` when every record is kept.
fn record_lost(snapshots: &[Evidence], ended: &BTreeSet<Ended>) -> Option<String> {
    let mut known: BTreeSet<Record> = BTreeSet::new();
    for (at, s) in snapshots.iter().enumerate() {
        known.extend(s.records.iter().copied());
        let marks = s.marks.clone().unwrap_or_default();
        for &(pid, lsn, kind) in &known {
            let listed = s.full || s.list.as_ref().is_some_and(|l| l.contains(&pid));
            let held = s.records.contains(&(pid, lsn, kind)) && s.redo.is_none_or(|r| lsn >= r);
            let ends = |e: &&Ended| e.0 == pid && e.1 > lsn && (!e.2 || kind != SETTLED);
            let gone = ended.iter().chain(&s.ended).any(|e| ends(&e));
            let covered = s.list.is_none() && marks.contains(&Quarantine::Database);
            if !(listed || held || gone || covered) {
                return Some(format!(
                    "observation {at}: page {} recorded at {lsn} as kind {kind}; list {:?}, redo \
                     {:?}, marks {:?}",
                    pid.0, s.list, s.redo, s.marks
                ));
            }
        }
    }
    None
}

/// Returns where the first mark the marks page of an observation of `snapshots` held is gone at a
/// later one, `None` when every mark stays.
fn mark_lost(snapshots: &[Evidence]) -> Option<String> {
    let mut held: BTreeSet<Quarantine> = BTreeSet::new();
    for (at, s) in snapshots.iter().enumerate() {
        let now = s.marks.clone().unwrap_or_default();
        let gone: BTreeSet<_> = held.difference(&now).collect();
        if !gone.is_empty() {
            return Some(format!(
                "observation {at}: {gone:?} gone; the page holds {:?}",
                s.marks
            ));
        }
        held.extend(now);
    }
    None
}

/// Returns the reads of the consumers that a mark durable before them should have refused and
/// did not: every read under the database's, and the read of a relation under its heap's.
/// `snapshots` ends with the consumers' own.
fn durable_marks_unmet(f: &Facts<'_>, snapshots: &[Evidence]) -> Vec<(String, String)> {
    let before = &snapshots[..snapshots.len().saturating_sub(1)];
    let durable: BTreeSet<Quarantine> = before
        .iter()
        .filter_map(|s| s.marks.clone())
        .flatten()
        .collect();
    let ids = &f.case.plan.ids;
    let mut out = Vec::new();
    for (rel, read) in &f.case.observed.reads {
        let marked = |m: &Quarantine| {
            *m == Quarantine::Database || ids.marks.get(rel).is_some_and(|(heap, _)| heap == m)
        };
        let Some(mark) = durable.iter().find(|m| marked(m)) else {
            continue;
        };
        let refused = match read {
            Read::Refused(what) => refused_marks(f, what)
                .iter()
                .any(|m| *m == *mark || *m == Quarantine::Database),
            Read::Failed(_) => true,
            Read::Rows(_) | Read::Garbage(_) => false,
        };
        if !refused {
            out.push((
                "a read serves past a durable mark".into(),
                format!("{}: {read:?}; durable {durable:?}", rel.name()),
            ));
        }
    }
    out
}

#[test]
fn a_record_recycled_with_its_log_before_any_fold_is_reported_lost() {
    let record = (Pid(9), 500, 1);
    let held = Evidence {
        records: BTreeSet::from([record]),
        redo: Some(400),
        list: Some(BTreeSet::new()),
        marks: Some(BTreeSet::new()),
        ..Evidence::default()
    };
    let recycled = Evidence {
        records: BTreeSet::new(),
        redo: Some(900),
        ..held.clone()
    };
    assert_eq!(
        Some(
            "observation 1: page 9 recorded at 500 as kind 1; list Some({}), redo Some(900), \
             marks Some({})"
                .to_owned()
        ),
        record_lost(&[held.clone(), recycled], &BTreeSet::new())
    );
    let bounded = Evidence {
        redo: Some(500),
        ..held.clone()
    };
    assert_eq!(None, record_lost(&[held, bounded], &BTreeSet::new()));
}

#[test]
fn a_marks_page_written_back_over_a_lost_mark_is_reported() {
    let marked = Evidence {
        marks: Some(BTreeSet::from([Quarantine::Heap(7)])),
        ..Evidence::default()
    };
    let written_back = Evidence {
        marks: Some(BTreeSet::from([Quarantine::Index(3)])),
        ..Evidence::default()
    };
    assert_eq!(
        Some("observation 1: {Heap(7)} gone; the page holds Some({Index(3)})".to_owned()),
        mark_lost(&[marked.clone(), written_back])
    );
    let kept = Evidence {
        marks: Some(BTreeSet::from([Quarantine::Heap(7), Quarantine::Index(3)])),
        ..Evidence::default()
    };
    assert_eq!(None, mark_lost(&[marked, kept]));
}
