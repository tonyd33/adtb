//! Act on chosen ops of a simulation: fail, lose, hold or run a closure at them, and number, count
//! and log them.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use adtb::runtime::{Instant, IoError, Op, OpKind, Ticket};

/// A set of ops, chosen by kind, by the path an op names and by its other arguments.
///
/// The path of an op on an open file is the path it was opened at; the path of a `Rename` is its
/// new name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Match {
    kinds: Option<Vec<OpKind>>,
    paths: Vec<Text>,
    from: Vec<Text>,
    offset: Option<Offset>,
    creating: bool,
    any_of: Vec<Match>,
    except: Vec<Match>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Text {
    Is(String),
    Prefixed(String),
    Suffixed(String),
    Containing(String),
}

impl Text {
    fn holds(&self, s: &str) -> bool {
        match self {
            Text::Is(want) => s == want,
            Text::Prefixed(prefix) => s.starts_with(prefix.as_str()),
            Text::Suffixed(suffix) => s.ends_with(suffix.as_str()),
            Text::Containing(part) => s.contains(part.as_str()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Offset {
    At(u64),
    Covers(u64),
    Whole(u64),
}

impl Match {
    pub fn any() -> Match {
        Match::default()
    }

    pub fn kind(kind: OpKind) -> Match {
        Match::kinds([kind])
    }

    pub fn kinds(kinds: impl IntoIterator<Item = OpKind>) -> Match {
        Match {
            kinds: Some(kinds.into_iter().collect()),
            ..Match::any()
        }
    }

    /// Returns the match of the ops any of `matches` matches.
    pub fn any_of(matches: impl IntoIterator<Item = Match>) -> Match {
        Match {
            any_of: matches.into_iter().collect(),
            ..Match::any()
        }
    }

    /// Returns this match limited to ops whose path is `path`.
    #[must_use]
    pub fn path(self, path: &str) -> Match {
        self.on_path(Text::Is(path.to_owned()))
    }

    /// Returns this match limited to ops whose path starts with `prefix`.
    #[must_use]
    pub fn prefixed(self, prefix: &str) -> Match {
        self.on_path(Text::Prefixed(prefix.to_owned()))
    }

    /// Returns this match limited to ops whose path ends with `suffix`.
    #[must_use]
    pub fn suffixed(self, suffix: &str) -> Match {
        self.on_path(Text::Suffixed(suffix.to_owned()))
    }

    /// Returns this match limited to ops whose path contains `part`.
    #[must_use]
    pub fn containing(self, part: &str) -> Match {
        self.on_path(Text::Containing(part.to_owned()))
    }

    fn on_path(mut self, test: Text) -> Match {
        self.paths.push(test);
        self
    }

    /// Returns this match limited to renames whose source path contains `part`.
    #[must_use]
    pub fn from_containing(mut self, part: &str) -> Match {
        self.from.push(Text::Containing(part.to_owned()));
        self
    }

    /// Returns this match limited to reads and writes at `offset`.
    #[must_use]
    pub fn offset(self, offset: u64) -> Match {
        self.at(Offset::At(offset))
    }

    /// Returns this match limited to reads and writes whose range holds `offset`.
    #[must_use]
    pub fn covers(self, offset: u64) -> Match {
        self.at(Offset::Covers(offset))
    }

    /// Returns this match limited to reads and writes of whole units of `unit` bytes: an offset
    /// and a length that are multiples of `unit`.
    #[must_use]
    pub fn whole(self, unit: u64) -> Match {
        self.at(Offset::Whole(unit))
    }

    fn at(self, offset: Offset) -> Match {
        Match {
            offset: Some(offset),
            ..self
        }
    }

    /// Returns this match limited to opens that create a missing file.
    #[must_use]
    pub fn creating(self) -> Match {
        Match {
            creating: true,
            ..self
        }
    }

    /// Returns this match without the ops `other` matches.
    #[must_use]
    pub fn except(mut self, other: Match) -> Match {
        self.except.push(other);
        self
    }

    fn matches(&self, op: &Op, path: Option<&str>) -> bool {
        if let Some(kinds) = &self.kinds
            && !kinds.contains(&op.kind())
        {
            return false;
        }
        let on_path = self
            .paths
            .iter()
            .all(|test| path.is_some_and(|p| test.holds(p)));
        let from = self.from.is_empty()
            || matches!(op, Op::Rename { from, .. } if self.from.iter().all(|t| t.holds(from)));
        let at = self.offset.is_none_or(|want| match op {
            Op::Read { offset, buf, .. } | Op::Write { offset, buf, .. } => {
                let len = buf.len() as u64;
                match want {
                    Offset::At(at) => *offset == at,
                    Offset::Covers(at) => *offset <= at && at < offset + len,
                    Offset::Whole(unit) => offset.is_multiple_of(unit) && len.is_multiple_of(unit),
                }
            }
            _ => false,
        });
        let creating = !self.creating || matches!(op, Op::Open { create: true, .. });
        on_path
            && from
            && at
            && creating
            && (self.any_of.is_empty() || self.any_of.iter().any(|m| m.matches(op, path)))
            && !self.except.iter().any(|m| m.matches(op, path))
    }

    fn is_kinds_only(&self) -> bool {
        self.paths.is_empty()
            && self.from.is_empty()
            && self.offset.is_none()
            && !self.creating
            && self.any_of.is_empty()
            && self.except.is_empty()
    }

    /// Returns whether this match takes writes alone.
    fn only_writes(&self) -> bool {
        let kinds = self
            .kinds
            .as_ref()
            .is_some_and(|kinds| kinds.iter().all(|k| *k == OpKind::Write));
        kinds || (!self.any_of.is_empty() && self.any_of.iter().all(Match::only_writes))
    }

    /// Returns whether this match may match a `Close`.
    fn may_close(&self) -> bool {
        let kind = self
            .kinds
            .as_ref()
            .is_none_or(|kinds| kinds.contains(&OpKind::Close));
        let alternative = self.any_of.is_empty() || self.any_of.iter().any(Match::may_close);
        let excluded = self.except.iter().any(|m| {
            m.is_kinds_only() && m.kinds.as_ref().is_some_and(|k| k.contains(&OpKind::Close))
        });
        kind && alternative
            && self.from.is_empty()
            && self.offset.is_none()
            && !self.creating
            && !excluded
    }
}

/// What the simulator does with a planned op.
pub enum Action {
    /// Complete the op with the error, changing nothing.
    Fail(IoError),
    /// Complete a write as written, changing nothing.
    Lose,
    /// Run the closure on the submitting thread, then queue the op as usual.
    Run(Box<dyn FnOnce() + Send>),
    /// Never complete the op.
    Hold,
}

impl Action {
    fn planned(&self) -> Planned {
        match self {
            Action::Fail(error) => Planned::Fail(*error),
            Action::Lose => Planned::Lose,
            Action::Run(_) => Planned::Run,
            Action::Hold => Planned::Hold,
        }
    }

    fn again(&self) -> Action {
        match self {
            Action::Fail(error) => Action::Fail(*error),
            Action::Lose => Action::Lose,
            Action::Hold => Action::Hold,
            Action::Run(_) => unreachable!("a run acts once"),
        }
    }
}

impl fmt::Debug for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.planned())
    }
}

/// The action a plan took at an op, without its closure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Planned {
    Fail(IoError),
    Lose,
    Run,
    Hold,
}

#[derive(Debug)]
struct Rule {
    ops: Match,
    action: Action,
    once: bool,
    lasting: bool,
}

/// Rules for every op that matches them, and actions at numbered ops.
#[derive(Debug, Default)]
pub struct Plan {
    rules: Vec<Rule>,
    numbered: BTreeMap<u64, Action>,
}

impl Plan {
    pub fn new() -> Plan {
        Plan::default()
    }

    /// Returns this plan acting with `action` on each op `ops` takes that no earlier rule takes.
    ///
    /// Preconditions:
    /// - `action` is no `Run`, since a run acts once.
    /// - `action` is no `Fail` when `ops` may take a `Close`.
    #[must_use]
    pub fn rule(self, ops: Match, action: Action) -> Plan {
        assert!(
            !matches!(action, Action::Run(_)),
            "a run acts once: plan it with first or at"
        );
        self.add(ops, action, false)
    }

    /// Returns this plan acting with `action` on the first op `ops` takes that no earlier rule
    /// does.
    ///
    /// Preconditions:
    /// - `action` is no `Fail` when `ops` may take a `Close`.
    #[must_use]
    pub fn first(self, ops: Match, action: Action) -> Plan {
        self.add(ops, action, true)
    }

    fn add(mut self, ops: Match, action: Action, once: bool) -> Plan {
        assert!(
            !(matches!(action, Action::Fail(_)) && ops.may_close()),
            "a failed close leaves its file open: except Close from {ops:?}"
        );
        assert!(
            !matches!(action, Action::Lose) || ops.only_writes(),
            "only a write can be lost: limit {ops:?} to writes"
        );
        self.rules.push(Rule {
            ops,
            action,
            once,
            lasting: false,
        });
        self
    }

    /// Returns this plan with its rules lasting through the later processes of the simulation.
    #[must_use]
    pub fn lasting(mut self) -> Plan {
        for rule in &mut self.rules {
            rule.lasting = true;
        }
        self
    }

    /// Returns this plan acting with `action` at op number `at` of the armed ops.
    #[must_use]
    pub fn at(mut self, at: u64, action: Action) -> Plan {
        self.numbered.insert(at, action);
        self
    }
}

/// The first numbered op an action was taken at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Struck {
    at: u64,
    kind: OpKind,
    path: Option<String>,
}

impl Struck {
    pub fn at(&self) -> u64 {
        self.at
    }

    pub fn kind(&self) -> OpKind {
        self.kind
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
}

impl fmt::Display for Struck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} of {:?}", self.kind, self.path)
    }
}

/// The ops numbered since the simulation first armed.
#[derive(Clone, Debug, Default)]
pub struct Numbered {
    kinds: Vec<OpKind>,
    struck: Option<Struck>,
    armed_at: usize,
}

impl Numbered {
    /// Returns the kind of each numbered op, by number.
    pub fn kinds(&self) -> &[OpKind] {
        &self.kinds
    }

    pub fn struck(&self) -> Option<&Struck> {
        self.struck.as_ref()
    }

    /// Returns the number of device events recorded when the last arm came.
    pub fn armed_at(&self) -> usize {
        self.armed_at
    }
}

/// A running count of the ops a [`Match`] accepts.
#[derive(Clone, Debug, Default)]
pub struct Count(Arc<AtomicU64>);

impl Count {
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// One op submitted, as a [`OpLog`] holds it.
#[derive(Clone, Debug)]
pub struct Submitted {
    ticket: Ticket,
    at: Instant,
    completed: Option<Instant>,
    kind: OpKind,
    path: Option<String>,
    from: Option<String>,
    offset: Option<u64>,
    len: Option<u64>,
    bytes: Option<Vec<u8>>,
    recorded: usize,
    planned: Option<Planned>,
}

impl Submitted {
    pub fn ticket(&self) -> Ticket {
        self.ticket
    }

    /// Returns the time the op was submitted.
    pub fn at(&self) -> Instant {
        self.at
    }

    /// Returns the time the op completed, or `None` while it has not.
    pub fn completed(&self) -> Option<Instant> {
        self.completed
    }

    pub fn kind(&self) -> OpKind {
        self.kind
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// Returns the source path of a rename.
    pub fn from(&self) -> Option<&str> {
        self.from.as_deref()
    }

    /// Returns the offset of a read or write.
    pub fn offset(&self) -> Option<u64> {
        self.offset
    }

    /// Returns the length of a read or write.
    pub fn length(&self) -> Option<u64> {
        self.len
    }

    /// Returns the bytes of a write.
    pub fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }

    /// Returns the number of device events recorded when the op was submitted.
    pub fn recorded(&self) -> usize {
        self.recorded
    }

    /// Returns the action the plan took at the op.
    pub fn planned(&self) -> Option<Planned> {
        self.planned
    }
}

/// The ops a [`Match`] accepts, in the order they were submitted.
#[derive(Clone, Debug, Default)]
pub struct OpLog(Arc<Mutex<Vec<Submitted>>>);

impl OpLog {
    pub fn entries(&self) -> Vec<Submitted> {
        self.lock().clone()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Submitted>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Append `entry`. Returns its place in this log.
    pub(crate) fn push(&self, entry: Submitted) -> usize {
        let mut entries = self.lock();
        entries.push(entry);
        entries.len() - 1
    }

    pub(crate) fn complete(&self, at: usize, now: Instant) {
        self.lock()[at].completed = Some(now);
    }
}

/// What a plan does with one op: its action, and the logs it goes to.
pub(crate) struct Decision {
    action: Option<Action>,
    planned: Option<Planned>,
    logs: Vec<OpLog>,
}

impl Decision {
    pub(crate) fn take_action(&mut self) -> Option<Action> {
        self.action.take()
    }

    pub(crate) fn into_logs(self) -> Vec<OpLog> {
        self.logs
    }

    /// Returns the log entry of `op`, submitted at `at` as `ticket`.
    pub(crate) fn entry(
        &self,
        op: &Op,
        path: Option<String>,
        ticket: Ticket,
        at: Instant,
        recorded: usize,
    ) -> Submitted {
        let (offset, len, bytes) = match op {
            Op::Read { offset, buf, .. } => (Some(*offset), Some(buf.len() as u64), None),
            Op::Write { offset, buf, .. } => {
                (Some(*offset), Some(buf.len() as u64), Some(buf.to_vec()))
            }
            _ => (None, None, None),
        };
        Submitted {
            ticket,
            at,
            completed: None,
            kind: op.kind(),
            path,
            from: match op {
                Op::Rename { from, .. } => Some(from.clone()),
                _ => None,
            },
            offset,
            len,
            bytes,
            recorded,
            planned: self.planned,
        }
    }
}

/// What a simulation applies to the ops submitted in it. Its rules last until the end of the run
/// that set them, unless planned as lasting; its numbering, counts and logs last as long as the
/// simulation.
#[derive(Debug, Default)]
pub(crate) struct Planner {
    rules: Vec<Rule>,
    numbered: BTreeMap<u64, Action>,
    owner: Option<&'static str>,
    armed: Option<Match>,
    seen: Numbered,
    counts: Vec<(Match, Count)>,
    logs: Vec<(Match, OpLog)>,
}

impl Planner {
    pub(crate) fn plan(&mut self, plan: Plan) {
        self.refuse("plan");
        self.rules = plan.rules;
        self.numbered = plan.numbered;
    }

    pub(crate) fn extend(&mut self, plan: Plan) {
        self.refuse("extend the plan");
        self.rules.extend(plan.rules);
        self.numbered.extend(plan.numbered);
    }

    /// Plan as `plan` states, and refuse any other plan for the life of the simulation.
    pub(crate) fn own(&mut self, owner: &'static str, plan: Plan) {
        self.plan(plan);
        self.owner = Some(owner);
    }

    fn refuse(&self, what: &str) {
        if let Some(owner) = self.owner {
            panic!("{owner} owns this simulation's plan: a workload under it does not {what}");
        }
    }

    pub(crate) fn end_run(&mut self) {
        self.rules.retain(|rule| rule.lasting);
    }

    pub(crate) fn arm(&mut self, ops: Match, recorded: usize) {
        self.armed = Some(ops);
        self.seen.armed_at = recorded;
    }

    /// Stop numbering and drop the numbered actions not taken. Returns how many ops were numbered.
    pub(crate) fn disarm(&mut self) -> u64 {
        self.armed = None;
        self.numbered.clear();
        self.seen.kinds.len() as u64
    }

    pub(crate) fn numbered(&self) -> Numbered {
        self.seen.clone()
    }

    pub(crate) fn counted(&mut self, ops: Match) -> Count {
        let count = Count::default();
        self.counts.push((ops, count.clone()));
        count
    }

    pub(crate) fn logged(&mut self, ops: Match) -> OpLog {
        let log = OpLog::default();
        self.logs.push((ops, log.clone()));
        log
    }

    /// Count `op`. Returns the action of the first rule that matches it or, when none does and it
    /// is armed, the action at its number, with the logs it goes to.
    pub(crate) fn decide(&mut self, op: &Op, path: Option<&str>) -> Decision {
        for (ops, count) in &self.counts {
            if ops.matches(op, path) {
                count.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let logs = self
            .logs
            .iter()
            .filter(|(ops, _)| ops.matches(op, path))
            .map(|(_, log)| log.clone())
            .collect();
        let action = self.act(op, path);
        Decision {
            planned: action.as_ref().map(Action::planned),
            action,
            logs,
        }
    }

    fn act(&mut self, op: &Op, path: Option<&str>) -> Option<Action> {
        if let Some(i) = self.rules.iter().position(|r| r.ops.matches(op, path)) {
            if self.rules[i].once {
                return Some(self.rules.remove(i).action);
            }
            return Some(self.rules[i].action.again());
        }
        if !self.armed.as_ref()?.matches(op, path) {
            return None;
        }
        let at = self.seen.kinds.len() as u64;
        self.seen.kinds.push(op.kind());
        let action = self.numbered.remove(&at)?;
        assert!(
            !(matches!(action, Action::Fail(_)) && op.kind() == OpKind::Close),
            "a failed close leaves its file open: op {at} is a Close"
        );
        self.seen.struck.get_or_insert_with(|| Struck {
            at,
            kind: op.kind(),
            path: path.map(str::to_owned),
        });
        Some(action)
    }
}
