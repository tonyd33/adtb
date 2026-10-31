//! What the quarantine marks stop: the owner of a damaged page, the read set a call is compared
//! to the marks by, and the plans that avoid a quarantined index.

use std::borrow::Cow;
use std::collections::BTreeSet;

use adtb_lang::kernel::{Kind, Terms, Tm};
use adtb_lang::schema::OnDelete;
use adtb_lang::types::{CtorSet, RelId, Ty};

use crate::btree::Index;
use crate::catalog::Holdings;
use crate::plan::{Catalog, HeapId, IndexId};
use crate::runtime::Runtime;
use crate::storage::{Damage, PageError, Pid, Quarantine};

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
    /// Returns what the damage at `pid` quarantines: the constructor heap or index holding the
    /// page, current or history, and otherwise the database. Every heap shares the one
    /// large-value store, so its pages fall to the database. The roots heap's damage marks
    /// nothing.
    fn holders(&self, pid: Pid) -> BTreeSet<Quarantine> {
        let mut holdings = Holdings::new(&self.schema);
        if let Some(kept) = &self.kept_store {
            holdings.roots(kept, None);
        }
        let retained = self.retained.as_deref();
        for r in self.rels.values() {
            for c in &r.ctors {
                let past = retained.and_then(|k| k.heaps.get(&c.heap));
                holdings.heaps(r.id, &self.heaps[&c.heap].heap, past.map(|p| &*p.heap));
            }
            for (id, _) in &r.indexes {
                let tree = self.indexes[id].index.tree();
                let past = retained.and_then(|k| k.indexes.get(id)).map(Index::tree);
                holdings.trees(r.id, tree.owner(), std::iter::once(tree).chain(past));
            }
        }
        self.pager.holders_of(pid, Some(Quarantine::Database), |h| {
            holdings.walk(&self.pager, h);
        })
    }

    /// Returns the name a client sees of `object`.
    pub(super) fn quarantined_name(&self, object: Quarantine) -> String {
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

    /// Mark `object`, ordered with every commit: a commit record written after the mark was
    /// checked against it, and a commit whose check missed it has its record in the log already.
    ///
    /// Precondition: the calling thread holds no page latch and not the serialization lock.
    fn set_mark(&self, object: Quarantine) -> Result<(), PageError> {
        let _ordered = self.ser.lock(&self.rt);
        self.pager.quarantine(object)
    }

    /// Mark `objects`. Returns the failure of the call that found them.
    ///
    /// Precondition: the calling thread holds no page latch.
    fn mark(&self, objects: BTreeSet<Quarantine>) -> Error {
        let mut names = Vec::new();
        for object in objects {
            let mut name = self.quarantined_name(object);
            if let Err(e) = self.set_mark(object) {
                name.push_str(&format!(" (the mark holds only in memory: {e:?})"));
            }
            names.push(name);
        }
        Error::Quarantined(names.join(", "))
    }

    /// Returns `e`, with damage to a page or the commit log marked and turned into the failure
    /// naming what is quarantined. An I/O failure stays as it is and sets no mark.
    ///
    /// Precondition: the calling thread holds no page latch.
    pub fn quarantining(&self, e: Error) -> Error {
        let damage = |p: &PageError| !matches!(p.damage, Damage::Unreadable);
        match e {
            Error::Page(p) if damage(&p) => {
                let held = self.holders(p.pid);
                if held.is_empty() {
                    Error::Page(p)
                } else {
                    self.mark(held)
                }
            }
            Error::Transactions(_) if self.txns.quarantined().as_ref().is_some_and(damage) => {
                self.mark(BTreeSet::from([Quarantine::Database]))
            }
            e => e,
        }
    }

    /// Mark what `found` implicates: the index of an `Order`, `Orphan` or `Missing` finding and
    /// the holder of a damaged page. Returns a finding for each mark that holds only in memory.
    pub fn quarantine_findings(&self, found: &BTreeSet<Found>) -> Vec<Found> {
        let mut objects = BTreeSet::new();
        for f in found {
            match f {
                Found::Order { index, .. }
                | Found::Orphan { index, .. }
                | Found::Missing { index, .. } => {
                    if let Some(i) = self.indexes.get(&IndexId(*index)) {
                        objects.insert(Quarantine::Index(i.index.tree().owner()));
                    }
                }
                Found::Checksum {
                    page: Some(pid), ..
                }
                | Found::StalePage { page: pid, .. } => objects.append(&mut self.holders(*pid)),
                _ => {}
            }
        }
        let mut failed = Vec::new();
        for object in objects {
            if let Err(e) = self.set_mark(object) {
                failed.push(Found::Structure {
                    at: format!(
                        "the mark of {} holds only in memory: {e:?}",
                        self.quarantined_name(object)
                    ),
                });
            }
        }
        failed
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
