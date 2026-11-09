//! What the quarantine marks stop: the owner of a damaged page, the read set a call is compared
//! to the marks by, and the plans that avoid a quarantined index.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::kernel::{Kind, Terms, Tm};
use adtb_lang::schema::OnDelete;
use adtb_lang::types::{CtorSet, RelId, Ty};

use crate::catalog::index_holder;
use crate::plan::{Catalog, HeapId, IndexId};
use crate::runtime::Runtime;
use crate::storage::policy::{Fault, Holder, Phase};
use crate::storage::{Damage, Ordered, Orders, PageError, Quarantine};

use super::schema::Rel;
use super::verify::Found;
use super::{Database, Error, Text, Txn};

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
                if object == Quarantine::Heap(self.heaps[&c.heap].heap.id().0) {
                    return format!("{}.{}", r.qualified, c.name);
                }
            }
            for (id, ix) in &r.indexes {
                if object == Quarantine::Index(self.indexes[id].index.tree().owner()) {
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
    fn map_damage(&self) -> BTreeMap<Quarantine, PageError> {
        self.pager.map_damage(
            Phase::Running,
            |pid| {
                let met = &mut BTreeSet::new();
                self.structures
                    .holders(&self.pager, &self.txns, pid, Phase::Running, met)
            },
            self,
        )
    }

    /// Map the damage recorded and not yet mapped for a commit, so that a mark its damage sets
    /// stops the commit. Leave out a record whose last mapping was incomplete, and map damage
    /// the walks detect to the database with no walk of its own.
    ///
    /// Precondition: the calling thread holds no page latch, and the serialization lock that
    /// `ordered` witnesses.
    pub(super) fn map_committing(&self, ordered: Ordered<'_>) {
        self.pager.map_committing(
            |pid, met| {
                self.structures
                    .holders(&self.pager, &self.txns, pid, Phase::Running, met)
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
        let owner = self.indexes.get(&id)?.index.tree().owner();
        Some(index_holder(&self.schema, r.id, owner))
    }

    /// Set what a finding on a page of `holder` marks, adding each mark that failed to `failed`.
    /// Returns the objects marked, none when the finding would hold a mark for an open only.
    ///
    /// Precondition: the calling thread holds no page latch and not the serialization lock.
    fn mark_finding(
        &self,
        holder: Holder,
        failed: &mut BTreeMap<Quarantine, PageError>,
    ) -> BTreeSet<Quarantine> {
        let mut objects = BTreeSet::new();
        let finding = self
            .pager
            .map_fault(Fault::Finding, holder, Phase::Running, self);
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

    /// Returns the failure naming `objects`, each failed mark of `failed` named with why.
    fn quarantined_as(
        &self,
        objects: &BTreeSet<Quarantine>,
        failed: &BTreeMap<Quarantine, PageError>,
    ) -> Error {
        let mut names = Vec::new();
        for object in objects {
            let mut name = self.quarantined_name(*object);
            if let Some(e) = failed.get(object) {
                name.push_str(&format!(" (the mark holds only in memory: {e:?})"));
            }
            names.push(name);
        }
        Error::Quarantined(names.join(", "))
    }

    /// Returns `e` once the damage recorded is mapped, a failure on a damaged page turned into
    /// the failure naming what its damage quarantines, and a failure of the transactions layer
    /// into the failure naming the database once the database is marked. An orphan entry marks
    /// its index. An I/O failure stays as it is.
    ///
    /// Precondition: the calling thread holds no page latch and not the serialization lock.
    pub fn quarantining(&self, e: Error) -> Error {
        let mut failed = self.map_damage();
        match e {
            Error::Page(p) if p.damage != Damage::Unreadable => match self.pager.mapped_to(p.pid) {
                Some(objects) if !objects.is_empty() => self.quarantined_as(&objects, &failed),
                _ => Error::Page(p),
            },
            Error::OrphanEntry(index) => {
                let id = self
                    .indexes
                    .iter()
                    .find(|(_, i)| i.index.tree().owner() == index);
                match id.and_then(|(id, _)| self.index_holder(*id)) {
                    Some(holder) => {
                        let objects = self.mark_finding(holder, &mut failed);
                        self.quarantined_as(&objects, &failed)
                    }
                    None => Error::OrphanEntry(index),
                }
            }
            Error::Transactions(_) if self.pager.is_quarantined(Quarantine::Database) => {
                self.quarantined_as(&BTreeSet::from([Quarantine::Database]), &failed)
            }
            e => e,
        }
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

    /// Fail if the database is quarantined.
    pub fn admit_database(&self) -> Result<(), Error> {
        if self.pager.is_quarantined(Quarantine::Database) {
            return Err(Error::Quarantined(
                self.quarantined_name(Quarantine::Database),
            ));
        }
        Ok(())
    }

    /// Fail if vacuum of `rel` would write a quarantined constructor heap or index.
    pub(super) fn admit_vacuum(&self, rel: &Rel) -> Result<(), Error> {
        let marks = self.pager.quarantined();
        let heaps = rel
            .ctors
            .iter()
            .map(|c| Quarantine::Heap(self.heaps[&c.heap].heap.id().0));
        let indexes = rel
            .indexes
            .iter()
            .map(|(id, _)| Quarantine::Index(self.indexes[id].index.tree().owner()));
        for object in std::iter::once(Quarantine::Database)
            .chain(heaps)
            .chain(indexes)
        {
            if marks.contains(&object) {
                return Err(Error::Quarantined(self.quarantined_name(object)));
            }
        }
        Ok(())
    }

    /// Fail if a constructor heap of `rel` is quarantined. The database's own mark plays no part.
    pub(crate) fn admit_heaps(&self, rel: RelId) -> Result<(), Error> {
        let marks = self.pager.quarantined();
        for c in &self.rels[&rel].ctors {
            let object = Quarantine::Heap(self.heaps[&c.heap].heap.id().0);
            if marks.contains(&object) {
                return Err(Error::Quarantined(self.quarantined_name(object)));
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

    /// Fail if the write set of `tx` is one [`Database::admit_touched`] refuses. What `tx` only
    /// read plays no part.
    ///
    /// Precondition: the calling thread holds the serialization lock, and the commit record of
    /// `tx` is written before it lets go.
    pub(super) fn admit_commit(&self, tx: &Txn) -> Result<(), Error> {
        let written = tx.written.take();
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
        let marked = |object: Quarantine| {
            if marks.contains(&object) {
                Err(Error::Quarantined(self.quarantined_name(object)))
            } else {
                Ok(())
            }
        };
        for heap in self.denial_heaps(&touched) {
            marked(Quarantine::Heap(heap.0 as u32))?;
        }
        for (r, c) in &touched.reads {
            if let Some(ctor) = self.rels.get(r).and_then(|rel| rel.ctors.get(*c as usize)) {
                marked(Quarantine::Heap(self.heaps[&ctor.heap].heap.id().0))?;
            }
        }
        for (r, c) in &touched.writes {
            let Some(rel) = self.rels.get(r) else {
                continue;
            };
            for (id, ix) in &rel.indexes {
                if ix.desc.ctors.contains(c) {
                    marked(Quarantine::Index(self.indexes[id].index.tree().owner()))?;
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

    /// Returns the heaps the constraint checks the writes in `touched` trigger read.
    fn denial_heaps(&self, touched: &Touched) -> BTreeSet<HeapId> {
        let relations: BTreeSet<RelId> = touched.writes.iter().map(|(r, _)| *r).collect();
        let mut heaps = BTreeSet::new();
        for d in self
            .denials
            .iter()
            .filter(|d| !d.reads.is_disjoint(&relations))
        {
            for (delta, _) in &d.deltas {
                let triggered = delta.trigger.is_none()
                    || delta.inputs.iter().any(|i| {
                        i.ctors
                            .iter()
                            .any(|c| touched.writes.contains(&(i.rel, *c)))
                    });
                if triggered {
                    heaps.extend(delta.plan.reads.heaps.iter().copied());
                }
            }
        }
        heaps
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
        for desc in catalog.relations.values_mut() {
            desc.indexes.retain(|i| {
                !self.indexes.get(&i.id).is_some_and(|stored| {
                    marks.contains(&Quarantine::Index(stored.index.tree().owner()))
                })
            });
        }
        Some(Cow::Owned(catalog))
    }
}

impl<R: Runtime> Orders for Database<R> {
    /// Run `f` under one hold of the serialization lock.
    ///
    /// Postconditions: a commit record written after a mark `f` sets was checked against it, and
    /// a commit whose check missed the mark has its record in the log already.
    fn ordered<T>(&self, f: impl FnOnce(Ordered<'_>) -> T) -> T {
        let mut ser = self.ser.lock(&self.rt);
        f(ser.ordered())
    }
}
