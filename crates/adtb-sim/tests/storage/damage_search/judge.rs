//! The judgement of one case from the test's own model: the rows the sequence acknowledged, the
//! pages the faults hit and the relation each write-out names. A case ends with the data intact,
//! with the damage reported and marked as the mapping of damage to marks says, or with a storage
//! failure the contract allows; anything else is a violation.

use adtb::testing::{Pid, Quarantine, page};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use adtb::Proof;

use super::case::{Fault, Plan};
use super::consumers::{Backup, Consumer, Copy, Dumped, Finding, Lookups, Observed, Read};
use super::ledger::{self, Ended, Evidence, Record, SETTLED};
use super::stated::{self, Class, Held, PHASES, Phase, Stated};
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
    pub ending: Ending,
    /// The damage records and the durable marks the file's evidence held, for a case judged on
    /// it.
    pub evidence: (usize, usize),
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
    /// The user relation holding the page, by its write-out's header.
    rel: Option<Rel>,
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
    /// The marks a case may show for the consumers' open alone: the database's, under a list
    /// page the device cannot read. No later open and no marks page holds them.
    open_marks: BTreeSet<Quarantine>,
    lasting: bool,
    unreadable: bool,
    /// Whether the first open met an unreadable page whose loss quarantines the database for
    /// that open only.
    open_only: bool,
    baseline: BTreeSet<Quarantine>,
    /// The pages the search could not classify, by their write-out.
    unclassified: Vec<String>,
    rows: BTreeSet<Row>,
}

impl Facts<'_> {
    fn primary(&self) -> Hit {
        self.case.hits[0]
    }

    /// Returns the marks the file holds beyond the sequence's deliberate damage and the marks
    /// dropped relations keep.
    fn marks(&self) -> BTreeSet<Quarantine> {
        let all = self.case.observed.marks.clone().unwrap_or_default();
        all.difference(&self.baseline)
            .filter(|m| !self.dropped_relation_keeps_its_mark(m))
            .copied()
            .collect()
    }

    /// Returns whether `m` is the mark of a relation the sequence dropped, set before the drop
    /// began.
    fn dropped_relation_keeps_its_mark(&self, m: &Quarantine) -> bool {
        let gone = |r: Rel| {
            !world::present(r, self.case.ran.acked.version) && !world::present(r, self.case.started)
        };
        self.case.plan.observed.ids.marked(*m).is_some_and(gone)
            && self.case.ran.marked_before_drop.contains(m)
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
    mark: Option<Quarantine>,
    may: BTreeSet<Quarantine>,
    /// Whether the page's damage quarantines the database for the first open alone.
    open: bool,
    /// The row of the phase the page's mapping surely ran in.
    row: Option<Row>,
}

/// Returns what the stated table says of `fault` on the page of write-out `w`, by the phases its
/// record may have been mapped in. The consumers read the marks in the session of the first open,
/// which maps once each record it holds; a later record is mapped once, at a call end or a
/// commit. An earlier session may have mapped the page in either phase only when the run held
/// its record. Returns nothing stated for a tear. Fails with the write-out of a page the search
/// cannot classify.
///
/// The search cannot see whether the first open's mapping of a record was complete: in a case
/// with an unreadable page, a page recorded at the open may be mapped at a later call end
/// instead, so its expected mark is the one both phases state alike.
fn judged(case: &Case<'_>, w: &WriteOut, fault: Fault) -> Result<Judged, String> {
    let o = case.observed;
    let ids = &case.plan.observed.ids;
    let holding = ids
        .holding(w.kind, w.owner, w.pid)
        .ok_or_else(|| format!("page {} of kind {} and owner {}", w.pid.0, w.kind, w.owner))?;
    let met_in_open = o.met_in_open && !is_lost(fault);
    let class = match fault {
        Fault::Torn => return Ok(Judged::default()),
        _ if o.settled.contains(&w.pid) => Class::Settled,
        Fault::LostEarlier | Fault::LostCurrent | Fault::Recycled => Class::Damaged,
        _ if met_in_open && holding.0 == Held::LsnMap => Class::Unarmed,
        Fault::UnreadableOnce | Fault::UnreadableLasting => Class::Unread,
    };
    let earlier = case.ran.recorded.contains(&w.pid);
    let incomplete = case
        .hits
        .iter()
        .any(|h| !is_lost(h.fault) && h.fault != Fault::Torn);
    let (surely, mut phases) = if met_in_open {
        (Some(Phase::Opening), vec![Phase::Opening])
    } else if o.recorded_at_open.contains(&w.pid) {
        let surely = (!incomplete || !is_lost(fault)).then_some(Phase::Opening);
        (surely, vec![Phase::Opening])
    } else {
        (Some(Phase::Running), vec![Phase::Running])
    };
    if earlier || surely.is_none() {
        phases = PHASES.to_vec();
    }
    let mark = |p| stated::durable(class, holding, p);
    let agreed = PHASES
        .iter()
        .map(|p| mark(*p))
        .reduce(|a, b| if a == b { a } else { None });
    Ok(Judged {
        mark: surely.map_or(agreed.flatten(), mark),
        may: phases.iter().filter_map(|p| mark(*p)).collect(),
        open: met_in_open && stated::stated(class, holding.0, Phase::Opening) == Stated::Open,
        row: surely.map(|p| (class, holding.0, p)),
    })
}

fn is_lost(fault: Fault) -> bool {
    matches!(
        fault,
        Fault::LostEarlier | Fault::LostCurrent | Fault::Recycled
    )
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

fn facts<'a>(case: &'a Case<'a>) -> Facts<'a> {
    let plan = case.plan;
    let ids = &plan.observed.ids;
    let mut pages = Vec::new();
    let mut unclassified = Vec::new();
    let mut open_only = false;
    for hit in &case.hits {
        let written = match hit.fault {
            Fault::UnreadableLasting => incarnation_met(case, hit.written),
            _ => hit.written,
        };
        let mut batch = vec![written];
        if is_lost(hit.fault) {
            let writes = &plan.clean.trace.writes;
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
            let rel = ids.owner(kind, w.owner);
            let found = page::lsn(&world::page(&case.observed.files, w.pid)).0;
            let stale = is_lost(hit.fault) && found < w.lsn;
            let alive = !freed(kind) || plan.held_at_end(w.pid);
            let must_report = matches!(hit.fault, Fault::LostCurrent | Fault::Recycled)
                && stale
                && alive
                && !unlogged_before_gap(case, &w, found);
            let must_not_name = case.ran.damaged != Some(w.pid)
                && match hit.fault {
                    Fault::Torn | Fault::UnreadableOnce => true,
                    Fault::LostEarlier => !stale,
                    _ => false,
                };
            let judged = judged(case, &w, hit.fault).unwrap_or_else(|e| {
                unclassified.push(e);
                Judged::default()
            });
            open_only |= judged.open;
            pages.push(Page {
                pid: w.pid,
                kind,
                fault: hit.fault,
                rel,
                stale,
                must_report,
                must_not_name,
                mark: judged.mark,
                may: judged.may,
                open: judged.open,
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
    let list_unreadable = case
        .hits
        .iter()
        .any(|h| h.fault == Fault::UnreadableLasting && h.kind == Kind::DamageList);
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
    let observed = case.observed.marks.is_some();
    let rows = pages
        .iter()
        .filter(|p| {
            let bites = p.must_report || p.open || named.contains(&p.pid);
            if matches!(p.fault, Fault::UnreadableOnce | Fault::UnreadableLasting) {
                bites || observed
            } else {
                p.stale && bites
            }
        })
        .filter_map(|p| p.row)
        .collect();
    let mut allowed_marks = lost_page_keeps_its_mark(&pages, &named);
    allowed_marks.extend(unused_slot_marks_its_index(&pages, ids));
    if open_only {
        allowed_marks.insert(Quarantine::Database);
    }
    let mut open_marks = BTreeSet::new();
    if list_unreadable && !allowed_marks.contains(&Quarantine::Database) {
        open_marks.insert(Quarantine::Database);
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
        open_marks,
        lasting,
        unreadable,
        open_only,
        baseline,
        unclassified,
        rows,
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
    let ids = &case.plan.observed.ids;
    let mut out = BTreeSet::new();
    if !lost.tail {
        out.insert(Quarantine::Database);
    }
    for w in past_lost(case, lost) {
        let object = ids
            .holding(w.kind, w.owner, w.pid)
            .and_then(|(_, object)| object);
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

/// Returns why the database is marked, by what the damaged pages are.
fn database_cause(f: &Facts<'_>) -> &'static str {
    let settled = &f.case.observed.settled;
    if f.pages.iter().all(|p| settled.contains(&p.pid)) {
        "for a settled status page"
    } else if f.pages.iter().any(|p| p.rel.is_some() && is_lost(p.fault)) {
        "the faults map to their relation"
    } else if f.pages.iter().all(|p| !is_lost(p.fault)) {
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
    for page in &f.unclassified {
        v(
            Consumer::Settle,
            "a damaged page the search cannot classify".into(),
            page.clone(),
        );
    }
    if let Some(Err(e)) = &o.open
        && !f.lasting
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
    if let Some(scrub) = &o.scrub {
        let named = !scrub.is_empty();
        let p = f
            .pages
            .iter()
            .find(|p| Some(p.pid) == o.scrubbed)
            .unwrap_or(&f.pages[0]);
        if p.must_report && !named {
            v(
                Consumer::Scrub,
                "a scrub misses the stale page".into(),
                String::new(),
            );
        }
        if p.must_not_name && named {
            v(
                Consumer::Scrub,
                "a scrub names an intact page".into(),
                scrub.join("; "),
            );
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
        rows: f.rows.clone(),
        reached: reached(&f),
        ending: ending(&f),
        evidence: held,
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
    let ids = &f.case.plan.observed.ids;
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
                p.fault == Fault::UnreadableLasting && (!relational(p.kind) || p.rel == Some(rel))
            });
            (!explained).then(|| (format!("{prefix}{name} fails"), e.clone()))
        }
    }
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
/// beyond those the stated table gives the pages, but the open's own quarantine of the database, which the next open with the page readable no
/// longer holds while it serves every row. Else every mark the stale pages map to, unless the
/// database's mark, which covers every object, is set and allowed, or an unreadable page leaves
/// the mapping incomplete; and no mark beyond those the case may show.
fn judge_marks(f: &Facts<'_>) -> Vec<(String, String)> {
    let marks = f.marks();
    let mut out = Vec::new();
    if f.unreadable && !f.case.hits.iter().any(|h| is_lost(h.fault)) {
        if f.open_only {
            let o = f.case.observed;
            let refused = o
                .reads
                .values()
                .chain(o.lookups.values().map(|l| &l.read))
                .all(|r| matches!(r, Read::Refused(what) if what == "the database"));
            if marks != BTreeSet::from([Quarantine::Database]) || !refused {
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
    let allowed: BTreeSet<Quarantine> = f.allowed_marks.union(&f.open_marks).copied().collect();
    let over: BTreeSet<_> = marks.difference(&allowed).collect();
    let durable = ledger::evidence(&f.case.observed.files).and_then(|e| e.marks);
    let reopened = f.case.observed.reopened.as_ref().map(|r| &r.marks);
    let outlived: BTreeSet<_> = f
        .open_marks
        .iter()
        .filter(|m| {
            reopened.is_some_and(|r| r.contains(m))
                || durable.as_ref().is_some_and(|d| d.contains(m))
        })
        .collect();
    if !outlived.is_empty() {
        out.push((
            "an unreadable list page leaves a mark past its open".into(),
            format!("{outlived:?}; reopened {reopened:?}; marks page {durable:?}"),
        ));
    }
    if !over.is_empty() {
        let shape = if !over.contains(&Quarantine::Database) {
            "a mark the faults do not map to"
        } else if f.pages.iter().any(|p| p.rel.is_some()) {
            "the database is marked for pages that map to their relation"
        } else if f
            .pages
            .iter()
            .all(|p| f.case.observed.settled.contains(&p.pid))
        {
            "the database is marked for a settled status page"
        } else {
            "a mark the faults do not map to"
        };
        let pages: Vec<_> = f.pages.iter().map(|p| (p.pid.0, p.stale)).collect();
        out.push((
            shape.into(),
            format!(
                "over {over:?}, expected {:?}; pages and staleness {pages:?}; settled {:?}",
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
    {
        out.push(("the backup fails".into(), e.clone()));
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
        out.push(("the dump fails".into(), e.clone()));
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
    let ids = &f.case.plan.observed.ids;
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
        (Consumer::Scrub, o.scrub.is_some()),
        (Consumer::Backup, o.backup.is_some()),
        (Consumer::Dump, o.dump.is_some()),
    ]
    .into_iter()
    .filter_map(|(c, ran)| ran.then_some(c))
    .collect()
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
    let ids = &f.case.plan.observed.ids;
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
