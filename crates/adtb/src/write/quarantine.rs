//! What the quarantine marks stop: the owner of a damaged page, the read set a call is compared
//! to the marks by, and the plans that avoid a quarantined index.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::kernel::{Kind, Terms, Tm};
use adtb_lang::schema::OnDelete;
use adtb_lang::types::{CtorSet, RelId, Ty};

use crate::catalog::index_holder;
use crate::plan::{Catalog, IndexId};
use crate::runtime::Runtime;
use crate::storage::policy::{Fault, Holder, Phase};
use crate::storage::{Damage, MarkFailure, Ordered, Orders, Quarantine};

use super::schema::Rel;
use super::verify::Found;
use super::{Database, Error, Text};

/// What made a call fail as quarantined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    /// The call met damage.
    Met,
    /// An index finding of the call refused it.
    Found,
    /// A mark refused the call.
    Admitted,
    /// The transactions layer failed once the database was marked.
    Transactions,
}

impl By {
    /// Returns whether the failure ends the transaction.
    pub fn ends_transaction(self) -> bool {
        match self {
            By::Met | By::Transactions => true,
            By::Found | By::Admitted => false,
        }
    }
}

/// A failure of the write path that [`Database::fail`] has mapped.
#[derive(Debug)]
pub struct Mapped(Error);

impl Mapped {
    pub fn error(&self) -> &Error {
        &self.0
    }

    pub fn into_error(self) -> Error {
        self.0
    }

    /// Returns `e` as if [`Database::fail`] had mapped it to itself.
    #[cfg(test)]
    pub(crate) fn unmapped_for_test(e: Error) -> Mapped {
        Mapped(e)
    }
}

impl Error {
    /// Returns the failure of a call that a mark of `object` refuses.
    pub(super) fn admitted(object: Quarantine) -> Error {
        Error::Quarantined {
            objects: BTreeMap::from([(object, None)]),
            by: By::Admitted,
        }
    }
}

/// Returns `objects` named by `name`, each mark that holds only in memory named with why.
pub(super) fn named(
    objects: &BTreeMap<Quarantine, Option<MarkFailure>>,
    name: impl Fn(Quarantine) -> String,
) -> String {
    let mut names = Vec::new();
    for (object, failed) in objects {
        let mut name = name(*object);
        if let Some(e) = failed {
            name.push_str(&format!(" (the mark holds only in memory: {e:?})"));
        }
        names.push(name);
    }
    names.join(", ")
}

/// What a call reads and writes, by constructor.
#[derive(Debug, Default)]
pub(super) struct Touched {
    reads: BTreeSet<(RelId, u32)>,
    writes: BTreeSet<(RelId, u32)>,
    /// The constructors a write stores a row into, whose references it checks.
    fills: BTreeSet<(RelId, u32)>,
    /// The relations a write may take a row or a constructor from.
    removes: BTreeSet<RelId>,
}

impl Touched {
    pub(super) fn wrote(&self) -> bool {
        !self.writes.is_empty()
    }
}

/// Returns the type of the elements of a set of type `ty`.
fn element(ty: &Ty) -> Option<&Ty> {
    match ty {
        Ty::Set(e) => Some(e),
        _ => None,
    }
}

impl<R: Runtime> Database<R> {
    /// Returns the name a client sees of `object`.
    pub(crate) fn quarantined_name(&self, object: Quarantine) -> String {
        for r in self.rels.values() {
            for c in &r.ctors {
                if object == Quarantine::Heap(self.bindings.heaps()[&c.heap].heap.id().0) {
                    return format!("{}.{}", r.qualified, c.name);
                }
            }
            for (id, ix) in &r.indexes {
                if object == Quarantine::Index(self.bindings.indexes()[id].index.tree().owner()) {
                    return format!("{} {}", r.qualified, ix.clause);
                }
            }
        }
        "the database".to_owned()
    }

    /// Map the damage recorded and not yet mapped, setting each mark under the serialization
    /// lock. Returns each object whose mark holds only in memory, with why.
    ///
    /// Precondition: the calling thread holds no page latch and not the serialization lock.
    fn map_damage(&self) -> BTreeMap<Quarantine, MarkFailure> {
        self.pager.map_damage(
            |pid| {
                self.structures
                    .holders(&self.pager, &self.session.txns, pid, Phase::Running)
            },
            self,
        )
    }

    /// Map the damage recorded and not yet mapped for a commit, so that a mark its damage sets
    /// stops the commit. Leave out a record whose last mapping was incomplete; damage the walks
    /// detect marks nothing of its own until a later mapping maps its record.
    ///
    /// Precondition: the calling thread holds no page latch, and the serialization lock that
    /// `ordered` witnesses.
    pub(super) fn map_committing(&self, ordered: Ordered<'_>) {
        self.pager.map_committing(
            |pid| {
                self.structures
                    .holders(&self.pager, &self.session.txns, pid, Phase::Running)
            },
            ordered,
        );
    }

    /// Returns the holder of the pages of index `id` and its history tree.
    pub(crate) fn index_holder(&self, id: IndexId) -> Option<Holder> {
        let r = self
            .rels
            .values()
            .find(|r| r.indexes.iter().any(|(i, _)| *i == id))?;
        let owner = self.bindings.indexes().get(&id)?.index.tree().owner();
        Some(index_holder(&self.schema, r.id, owner))
    }

    /// Set what a finding on a page of `holder` marks, adding each mark that failed to `failed`.
    /// Returns the objects marked, none when the finding would hold a mark for an open only.
    ///
    /// Precondition: the calling thread holds no page latch and not the serialization lock.
    fn mark_finding(
        &self,
        holder: Holder,
        failed: &mut BTreeMap<Quarantine, MarkFailure>,
    ) -> BTreeSet<Quarantine> {
        let mut objects = BTreeSet::new();
        let finding = self.pager.map_fault(Fault::Finding, holder, self);
        let Ok(marked) = finding else {
            return objects;
        };
        for (object, marked) in marked {
            objects.insert(object);
            if let Err(e) = marked {
                failed.insert(object, e);
            }
        }
        objects
    }

    /// Returns the name a client sees of `objects`, each mark that holds only in memory named
    /// with why.
    pub(crate) fn quarantined_names(
        &self,
        objects: &BTreeMap<Quarantine, Option<MarkFailure>>,
    ) -> String {
        named(objects, |object| self.quarantined_name(object))
    }

    /// Returns `e` as [`detail`](super::detail) reads it, a quarantined failure naming its
    /// objects as a client sees them.
    pub fn detail(&self, e: &Error) -> String {
        match e {
            Error::Quarantined { objects, .. } => {
                format!("Quarantined({:?})", self.quarantined_names(objects))
            }
            e => super::detail(e),
        }
    }

    /// Returns `e` once the damage recorded is mapped, a failure on a damaged page turned into
    /// the failure naming what its damage quarantines, and a failure of the transactions layer
    /// into the failure naming the database once the database is marked. An orphan entry marks
    /// its index. An I/O failure stays as it is. A `Blocked` maps nothing.
    ///
    /// Precondition: the calling thread holds no page latch and not the serialization lock.
    pub fn fail(&self, e: Error) -> Mapped {
        if let Error::Blocked { .. } = e {
            return Mapped(e);
        }
        let mut failed = self.map_damage();
        let quarantined_as = |objects: &BTreeSet<Quarantine>, failed: &BTreeMap<_, _>, by| {
            let objects = objects.iter().map(|o| (*o, failed.get(o).copied()));
            Error::Quarantined {
                objects: objects.collect(),
                by,
            }
        };
        Mapped(match e {
            Error::Page(p) if p.damage != Damage::Unreadable => match self.pager.marked_to(p.pid) {
                Some(objects) if !objects.is_empty() => quarantined_as(&objects, &failed, By::Met),
                _ => Error::Page(p),
            },
            Error::OrphanEntry(index) => {
                let id =
                    (self.bindings.indexes().iter()).find(|(_, i)| i.index.tree().owner() == index);
                match id.and_then(|(id, _)| self.index_holder(*id)) {
                    Some(holder) => {
                        let objects = self.mark_finding(holder, &mut failed);
                        quarantined_as(&objects, &failed, By::Found)
                    }
                    None => Error::OrphanEntry(index),
                }
            }
            Error::Transactions(_) if self.pager.is_quarantined(Quarantine::Database) => {
                quarantined_as(
                    &BTreeSet::from([Quarantine::Database]),
                    &failed,
                    By::Transactions,
                )
            }
            e => e,
        })
    }

    /// Mark the index of each `Order`, `Orphan` or `Missing` finding of `found`, and map the
    /// damage recorded. Returns a finding for each mark that holds only in memory.
    ///
    /// Precondition: the calling thread holds no page latch and not the serialization lock.
    pub fn quarantine_findings(&self, found: &BTreeSet<Found>) -> Vec<Found> {
        let mut failed = self.map_damage();
        for f in found {
            if let Found::Order { index, .. }
            | Found::Orphan { index, .. }
            | Found::Missing { index, .. } = f
                && let Some(holder) = self.index_holder(IndexId(*index))
            {
                self.mark_finding(holder, &mut failed);
            }
        }
        failed
            .into_iter()
            .map(|(object, e)| Found::Structure {
                at: format!(
                    "the mark of {} holds only in memory: {e:?}",
                    self.quarantined_name(object)
                ),
            })
            .collect()
    }

    /// Returns the refusal [`admit_database`](Database::admit_database) gives, as
    /// [`fail`](Database::fail) maps it, without mapping the damage recorded. `None` when the
    /// database is not quarantined.
    pub fn database_refusal(&self) -> Option<Mapped> {
        self.admit_database().err().map(Mapped)
    }

    /// Fail if the database is quarantined.
    pub fn admit_database(&self) -> Result<(), Error> {
        if self.pager.is_quarantined(Quarantine::Database) {
            return Err(Error::admitted(Quarantine::Database));
        }
        Ok(())
    }

    /// Fail if vacuum of `rel` would write a quarantined constructor heap or index.
    pub(super) fn admit_vacuum(&self, rel: &Rel) -> Result<(), Error> {
        let marks = self.pager.quarantined();
        let heaps = rel
            .ctors
            .iter()
            .map(|c| Quarantine::Heap(self.bindings.heaps()[&c.heap].heap.id().0));
        let indexes = rel
            .indexes
            .iter()
            .map(|(id, _)| Quarantine::Index(self.bindings.indexes()[id].index.tree().owner()));
        for object in std::iter::once(Quarantine::Database)
            .chain(heaps)
            .chain(indexes)
        {
            if marks.contains(&object) {
                return Err(Error::admitted(object));
            }
        }
        Ok(())
    }

    /// Fail if a constructor heap of `rel` is quarantined. The database's own mark plays no part.
    pub(crate) fn admit_heaps(&self, rel: RelId) -> Result<(), Error> {
        let marks = self.pager.quarantined();
        for c in &self.rels[&rel].ctors {
            let object = Quarantine::Heap(self.bindings.heaps()[&c.heap].heap.id().0);
            if marks.contains(&object) {
                return Err(Error::admitted(object));
            }
        }
        Ok(())
    }

    /// Fail if a row of `rel` cannot be written: the database, a constructor heap or an index of
    /// `rel` is quarantined. A key spans the sum, so a mark on any constructor counts.
    pub fn admit_relation(&self, rel: RelId) -> Result<(), Error> {
        self.admit_vacuum(&self.rels[&rel])
    }

    /// Fail if the items of `text` read a quarantined constructor relation, write one a
    /// quarantined index covers, or the database is quarantined. Their plans and the values of
    /// their parameters play no part.
    pub(super) fn admit(&self, text: &Text<'_>) -> Result<(), Error> {
        let marks = self.pager.quarantined();
        if marks.is_empty() || text.items.is_empty() {
            return Ok(());
        }
        let mut touched = Touched::default();
        for &t in text.items {
            self.walk(text.terms, t, &mut touched);
        }
        self.admit_touched(&marks, touched)
    }

    /// Fail if `written`, the write set of a committing transaction, is one
    /// [`Database::admit_touched`] refuses. What the transaction only read plays no part.
    ///
    /// Precondition: the calling thread holds the serialization lock, and the commit record of
    /// the transaction is written before it lets go.
    pub(super) fn admit_commit(&self, written: Touched) -> Result<(), Error> {
        if written.writes.is_empty() || !self.pager.any_quarantined() {
            return Ok(());
        }
        self.admit_touched(&self.pager.quarantined(), written)
    }

    /// Fail if `touched` reads a quarantined constructor relation, writes one a quarantined index
    /// covers, its checks read a quarantined constructor relation, or the database is
    /// quarantined.
    fn admit_touched(
        &self,
        marks: &BTreeSet<Quarantine>,
        mut touched: Touched,
    ) -> Result<(), Error> {
        self.admit_database()?;
        self.follow_writes(&mut touched);
        self.follow_checks(&mut touched);
        let marked = |object: Quarantine| {
            if marks.contains(&object) {
                Err(Error::admitted(object))
            } else {
                Ok(())
            }
        };
        for (r, c) in &touched.reads {
            if let Some(ctor) = self.rels.get(r).and_then(|rel| rel.ctors.get(*c as usize)) {
                marked(Quarantine::Heap(
                    self.bindings.heaps()[&ctor.heap].heap.id().0,
                ))?;
            }
        }
        for (r, c) in &touched.writes {
            let Some(rel) = self.rels.get(r) else {
                continue;
            };
            for (id, ix) in &rel.indexes {
                if ix.desc.ctors.contains(c) {
                    marked(Quarantine::Index(
                        self.bindings.indexes()[id].index.tree().owner(),
                    ))?;
                }
            }
        }
        Ok(())
    }

    /// Add to `written` what the outcome `o` of a statement wrote into the file.
    pub(super) fn record_written(&self, written: &mut Touched, o: &super::Outcome) {
        for &(r, c) in o.inserted.keys().filter(|(r, _)| self.rels.contains_key(r)) {
            written.fills.insert((r, c));
            written.writes.insert((r, c));
        }
        for &(r, c) in o.deleted.keys().filter(|(r, _)| self.rels.contains_key(r)) {
            written.removes.insert(r);
            written.writes.insert((r, c));
        }
    }

    /// Add what term `root` of `terms`, a normal form with every function inlined, reads and writes.
    fn walk(&self, terms: &Terms, root: Tm, out: &mut Touched) {
        let mut seen = BTreeSet::new();
        let mut stack = vec![root];
        while let Some(t) = stack.pop() {
            if !seen.insert(t) {
                continue;
            }
            let kind = terms.kind(t);
            self.term_facts(terms, kind, out);
            stack.extend(kind.children());
        }
    }

    fn term_facts(&self, terms: &Terms, kind: &Kind, out: &mut Touched) {
        match kind {
            Kind::Rel(r, set) | Kind::Transition(_, r, set) => {
                self.add_reads(*r, set.as_ref(), out);
            }
            Kind::Deref { key, .. } => {
                if let Ty::Key { rel, ctors } = terms.ty(*key) {
                    self.add_reads(*rel, ctors.as_ref(), out);
                }
            }
            Kind::Insert(r, rows) | Kind::Upsert { rel: r, rows, .. } => {
                self.add_fills(*r, element(terms.ty(*rows)), out);
                if matches!(kind, Kind::Upsert { .. }) {
                    out.removes.insert(*r);
                }
            }
            Kind::Delete(r, rows) => {
                self.add_writes(*r, element(terms.ty(*rows)), out);
                out.removes.insert(*r);
            }
            Kind::Replace(r, rows, f) => {
                self.add_writes(*r, element(terms.ty(*rows)), out);
                match terms.ty(*f) {
                    Ty::Fn(_, new) => self.add_fills(*r, Some(&**new), out),
                    _ => self.add_fills(*r, None, out),
                }
                out.removes.insert(*r);
            }
            Kind::Rejects { into, .. } => self.add_fills(*into, None, out),
            _ => {}
        }
    }

    fn add_reads(&self, r: RelId, set: Option<&CtorSet>, out: &mut Touched) {
        match set {
            Some(set) => out.reads.extend(set.iter().map(|c| (r, *c))),
            None => {
                if let Some(rel) = self.rels.get(&r) {
                    out.reads
                        .extend(rel.all_ctors().into_iter().map(|c| (r, c)));
                }
            }
        }
    }

    /// Add a write into `r` of rows of type `row`: into its constructors, else into every
    /// constructor of `r`.
    fn add_writes(&self, r: RelId, row: Option<&Ty>, out: &mut Touched) {
        match row {
            Some(Ty::Data {
                ctors: Some(set), ..
            }) => out.writes.extend(set.iter().map(|c| (r, *c))),
            _ => {
                if let Some(rel) = self.rels.get(&r) {
                    out.writes
                        .extend(rel.all_ctors().into_iter().map(|c| (r, c)));
                }
            }
        }
    }

    /// Add a write of rows of type `row` into `r` as [`Database::add_writes`] does, whose
    /// references are checked.
    fn add_fills(&self, r: RelId, row: Option<&Ty>, out: &mut Touched) {
        let mut one = Touched::default();
        self.add_writes(r, row, &mut one);
        out.fills.extend(one.writes.iter().copied());
        out.writes.append(&mut one.writes);
    }

    /// Add what the checks of the writes in `out` read: the sum whose one key a written row must
    /// not repeat, the targets of the references of a row it stores, and the referrers of a row
    /// it removes, with the writes their `on delete` actions make.
    fn follow_writes(&self, out: &mut Touched) {
        let mut removed: Vec<RelId> = out.removes.iter().copied().collect();
        while let Some(target) = removed.pop() {
            for rel in self.rels.values() {
                self.follow_referrers(rel, target, out, &mut removed);
            }
        }
        for (r, c) in out.writes.clone() {
            let Some(rel) = self.rels.get(&r) else {
                continue;
            };
            out.reads
                .extend(rel.all_ctors().into_iter().map(|d| (r, d)));
            if out.fills.contains(&(r, c))
                && let Some(ctor) = rel.ctors.get(c as usize)
            {
                for f in &ctor.refs {
                    self.add_reads(f.target, f.ctors.as_ref(), out);
                }
            }
        }
    }

    fn follow_referrers(
        &self,
        rel: &Rel,
        target: RelId,
        out: &mut Touched,
        removed: &mut Vec<RelId>,
    ) {
        for (c, ctor) in (0u32..).zip(&rel.ctors) {
            for f in ctor.refs.iter().filter(|f| f.target == target) {
                out.reads.insert((rel.id, c));
                let written = match f.action {
                    OnDelete::Restrict => None,
                    OnDelete::Cascade | OnDelete::Clear => Some(c),
                    OnDelete::Retag(to, _) => Some(to),
                };
                if let Some(to) = written {
                    out.writes.insert((rel.id, c));
                    out.writes.insert((rel.id, to));
                    if !matches!(f.action, OnDelete::Cascade) {
                        out.fills.insert((rel.id, to));
                    }
                    if out.removes.insert(rel.id) {
                        removed.push(rel.id);
                    }
                }
            }
        }
    }

    /// Add what the constraint checks the writes in `out` trigger read.
    fn follow_checks(&self, out: &mut Touched) {
        let relations: BTreeSet<RelId> = out.writes.iter().map(|(r, _)| *r).collect();
        let mut reads = Touched::default();
        for d in self
            .denials
            .iter()
            .filter(|d| !d.reads.is_disjoint(&relations))
        {
            for (delta, _) in &d.deltas {
                let triggered = delta.trigger.is_none()
                    || delta
                        .inputs
                        .iter()
                        .any(|i| i.ctors.iter().any(|c| out.writes.contains(&(i.rel, *c))));
                if triggered {
                    self.walk(&self.program, delta.term(), &mut reads);
                }
            }
        }
        out.reads.append(&mut reads.reads);
    }

    /// Returns the catalog a plan chooses its probes from now, with no quarantined index, or
    /// `None` when that is the whole catalog. A key lookup goes on naming its key index, and the
    /// store answers it from the heaps while the index is quarantined.
    pub(super) fn usable_catalog(&self) -> Option<Cow<'_, Catalog>> {
        let marks = self.pager.quarantined();
        if !marks.iter().any(|m| matches!(m, Quarantine::Index(_))) {
            return None;
        }
        let mut catalog = (*self.catalog).clone();
        catalog.retain_indexes(|i| {
            !self.bindings.indexes().get(&i.id).is_some_and(|stored| {
                marks.contains(&Quarantine::Index(stored.index.tree().owner()))
            })
        });
        Some(Cow::Owned(catalog))
    }
}

impl<R: Runtime> Orders for Database<R> {
    /// Run `f` under one hold of the serialization lock.
    ///
    /// Postconditions: a commit record written after a mark `f` sets was checked against it, and
    /// a commit whose check missed the mark has its record in the log already.
    fn ordered<T>(&self, f: impl FnOnce(Ordered<'_>) -> T) -> T {
        let mut ser = self.session.ser.lock(&self.rt);
        f(ser.ordered())
    }
}
