use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use adtb_lang::ast::{Decl, MigrationFile, OperatorKind};
use adtb_lang::check::Compiled;
use adtb_lang::evolve::{self, Kept, Source};
use adtb_lang::parse::{Options, parse_migration};
use adtb_lang::schema::{Schema, Stage, ValueDef};
use adtb_lang::span::{Diag, FileDiag, Span};
use adtb_lang::translate::operator_text;
use adtb_lang::types::{RelId, Ty, TypeId};

use super::Rot;
use super::ids::{
    CatConstraintId, CatCtorId, CatFieldId, CatIndexId, CatRelId, CatSetId, ctor_id, ctor_index,
    rel_id, relation, type_ctor_id,
};
use super::steps::Stepped;
use super::{Broken, Catalog, Checked, ElementTexts, Error, ctor_count};

/// A live reader of the database: a bundle with its feeds and registered change consumers, at
/// the version it is pinned to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reader {
    pub bundle: String,
    pub version: u64,
    /// The bundle's header declares `hide new constructors`.
    pub hides_new: bool,
    /// What its queries and feeds read, named as at `version`.
    pub reads: Vec<Read>,
    /// The constructor relations its queries and transactions can write into, each a qualified
    /// relation and a constructor of it, named as at `version`.
    pub writes: Vec<(String, String)>,
}

/// One thing a reader reads, its relations qualified as `module::name`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// A query ranges over the whole relation variable, or a feed names it whole. A feed or a
    /// query that reads its rows whole also reads each of their fields.
    Relation(String),
    /// A query follows a reference `Key relation`, or `Key relation.set` for a named set.
    Reference {
        relation: String,
        set: Option<String>,
    },
    /// A query reads the field of the rows of the relation.
    Field { relation: String, field: String },
}

/// A drop that has applied and whose object stays enforced while a live reader pinned to a
/// version holding the object remains.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(test)]
pub struct PendingDrop {
    /// The qualified name of the dropped constraint or check, or of the relation of a dropped
    /// unique or old key followed by its clause.
    pub object: String,
    /// The version the drop produced.
    pub dropped: u64,
    /// The bundles holding the drop back, in order.
    pub blocking: Vec<String>,
}

/// A reader with its names resolved to catalog ids.
#[derive(Clone, Debug)]
pub(super) struct Pinned {
    version: u64,
    hides_new: bool,
    whole: BTreeSet<CatRelId>,
    references: Vec<(CatRelId, Option<CatSetId>)>,
    fields: BTreeSet<CatFieldId>,
    writes: BTreeSet<(CatRelId, CatCtorId)>,
    /// The constructors that exist at `version`.
    ctors: BTreeSet<CatCtorId>,
    /// The constraints and checks that exist at `version`.
    constraints: BTreeSet<CatConstraintId>,
    /// The uniques that exist at `version`.
    uniques: BTreeSet<CatIndexId>,
    /// The relations that exist at `version`.
    relations: BTreeSet<CatRelId>,
}

/// What a pending drop holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Constraint(CatConstraintId),
    Check(CatConstraintId),
    Unique(CatIndexId),
    /// The old key of a relaxing rekey of `relation`, over `fields`, enforced by index `index`.
    Key {
        index: CatIndexId,
        relation: CatRelId,
        fields: Vec<CatFieldId>,
    },
}

impl Pinned {
    /// Returns how this reader reads constructor `c` of relation `r` of `schema` such that
    /// enabling it is refused, `None` when enabling it is not.
    fn blocks(&self, schema: &Schema, r: RelId, c: u32) -> Option<String> {
        let rid = rel_id(schema, r);
        if self.hides_new || self.ctors.contains(&ctor_id(schema, r, c)) {
            return None;
        }
        let rel = schema.relation(r);
        if self.whole.contains(&rid) {
            return Some(format!("reads {} whole", rel.name));
        }
        let follows = self
            .references
            .iter()
            .any(|&(to, set)| to == rid && set.is_none_or(|s| in_set(schema, r, s, c)));
        follows.then(|| {
            let ctor = &schema.ty(rel.elem_def).ctors()[c as usize].name;
            format!("follows a reference that admits {}.{ctor}", rel.name)
        })
    }

    /// Returns how this reader reads constructor `c` of type `def` of `schema` such that
    /// enabling it is refused, `None` when enabling it is not.
    fn blocks_value(&self, schema: &Schema, def: TypeId, c: u32) -> Option<String> {
        if self.ctors.contains(&type_ctor_id(schema, def, c)) {
            return None;
        }
        read_fields(schema, self)
            .any(|t| schema.reaches(t, def))
            .then(|| format!("reads a field whose type reaches {}", schema.ty(def).name))
    }
}

/// A pending drop. The `name` of a unique or an old key is its relation followed by its clause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Pending {
    pub(super) module: String,
    pub(super) name: String,
    pub(super) kind: Kind,
    pub(super) dropped: u64,
    pub(super) blocking: BTreeSet<String>,
}

impl Pending {
    /// Returns the drop of `kind`, `name` in `module`, that the migration whose result is
    /// `applying` makes and `blocking` holds back.
    fn of(
        applying: &Checked,
        (module, name): (String, String),
        kind: Kind,
        blocking: BTreeSet<String>,
    ) -> Pending {
        Pending {
            module,
            name,
            kind,
            dropped: applying.source.version(),
            blocking,
        }
    }

    pub(super) fn object(&self) -> String {
        format!("{}::{}", self.module, self.name)
    }

    /// Returns the declaration a migration keeps for this drop, `None` for a unique or an old key.
    pub(super) fn kept(&self) -> Option<Kept> {
        match self.kind {
            Kind::Constraint(id) | Kind::Check(id) => Some(Kept(id)),
            Kind::Unique(_) | Kind::Key { .. } => None,
        }
    }

    fn unique(&self) -> Option<CatIndexId> {
        match self.kind {
            Kind::Unique(id) => Some(id),
            Kind::Constraint(_) | Kind::Check(_) | Kind::Key { .. } => None,
        }
    }
}

/// A constraint, or a check when `check`, that a step of a migration drops.
#[derive(Clone, Debug)]
struct Dropped {
    id: CatConstraintId,
    module: String,
    name: String,
    check: bool,
}

/// A unique that operator `op` of a migration, a `drop unique`, drops.
#[derive(Clone, Debug)]
struct DroppedUnique {
    op: usize,
    id: CatIndexId,
    module: String,
    name: String,
}

/// What the steps of a migration do to write states and constraints, by catalog id, each with
/// the position of the first operator whose step does it.
#[derive(Debug, Default)]
struct Changes {
    /// The constructor relations a step makes writable.
    enabled: BTreeMap<(CatRelId, CatCtorId), usize>,
    /// The value-level constructors a step makes writable.
    value_enabled: BTreeMap<CatCtorId, usize>,
    /// The constructor relations a step stops writes into, with the verb that does so.
    stopped: BTreeMap<(CatRelId, CatCtorId, &'static str), usize>,
    dropped: Vec<Dropped>,
    uniques: Vec<DroppedUnique>,
    /// The uniques a step removes.
    lost_uniques: BTreeMap<CatIndexId, usize>,
    /// The constraints and checks a step adds, by module and name.
    added: BTreeMap<(String, String), usize>,
}

impl Changes {
    /// Record what the step from `a` to `b` of operator `op` changes.
    fn add(&mut self, op: usize, a: &Schema, b: &Schema) {
        for (r, rid) in b.ids().relation_ids() {
            let Some(was) = relation(a, rid) else {
                continue;
            };
            for c in 0..ctor_count(b, r) {
                let cid = ctor_id(b, r, c);
                let Some(k) = ctor_index(a, was, cid) else {
                    continue;
                };
                match (a.stage(was, k), b.stage(r, c)) {
                    (Stage::Enabled, Stage::Deprecated) => {
                        self.stopped.entry((rid, cid, "deprecate")).or_insert(op);
                    }
                    (Stage::Enabled, Stage::Declared) => {
                        self.stopped.entry((rid, cid, "disable")).or_insert(op);
                    }
                    (Stage::Declared | Stage::Deprecated, Stage::Enabled) => {
                        self.enabled.entry((rid, cid)).or_insert(op);
                    }
                    _ => {}
                }
            }
        }
        for (bc, id) in b.ids().ctor_ids() {
            if let Some(ac) = a.ids().ctor_of(id)
                && a.value_stage(ac.def(), ac.index()) != Stage::Enabled
                && b.value_stage(bc.def(), bc.index()) == Stage::Enabled
            {
                self.value_enabled.entry(id).or_insert(op);
            }
        }
        let kept: BTreeSet<CatConstraintId> = b
            .ids()
            .constraint_ids()
            .map(|(_, id)| id)
            .chain(b.ids().check_ids().map(|(_, id)| id))
            .collect();
        let kept_uniques = unique_ids(b);
        for id in unique_ids(a).difference(&kept_uniques) {
            self.lost_uniques.entry(*id).or_insert(op);
        }
        let names = |s: &Schema| -> BTreeSet<(String, String)> {
            let constraints = s.constraints.iter().map(|c| (c.module, c.name.clone()));
            let checks = s.checks.iter().map(|c| (c.module, c.name.clone()));
            constraints
                .chain(checks)
                .map(|(m, n)| (s.module(m).name.clone(), n))
                .collect()
        };
        for name in names(b).difference(&names(a)) {
            self.added.entry(name.clone()).or_insert(op);
        }
        let named = |m, name: &str, id, check| Dropped {
            id,
            module: a.module(m).name.clone(),
            name: name.to_string(),
            check,
        };
        for (i, id) in a.ids().constraint_ids() {
            if !kept.contains(&id) {
                let c = a.constraint(i);
                self.dropped.push(named(c.module, &c.name, id, false));
            }
        }
        for (i, id) in a.ids().check_ids() {
            if !kept.contains(&id) {
                let c = a.check(i);
                self.dropped.push(named(c.module, &c.name, id, true));
            }
        }
    }

    /// Record each unique the step from `a`, the schema of `source`, to `b` of operator `op`, a
    /// `drop unique`, drops.
    fn drop_uniques(
        &mut self,
        op: usize,
        source: &Source,
        a: &Schema,
        b: &Schema,
    ) -> Result<(), Error> {
        let kept = unique_ids(b);
        if unique_ids(a).is_subset(&kept) {
            return Ok(());
        }
        for (id, (r, clause)) in uniques(a, &ElementTexts::of(source, a)?) {
            if kept.contains(&id) {
                continue;
            }
            let rel = a.relation(r);
            self.uniques.push(DroppedUnique {
                op,
                id,
                module: a.module(rel.module).name.clone(),
                name: format!("{} {clause}", rel.name),
            });
        }
        Ok(())
    }
}

/// Returns what `stepped`, the steps of a migration's operators applied to `source`, whose
/// schema is `schema`, do to write states, constraints and uniques.
fn changes(source: &Source, schema: &Schema, stepped: &Stepped) -> Result<Changes, Error> {
    let mut before = schema;
    let mut before_source = source;
    let mut changes = Changes::default();
    for (i, (op, steps)) in stepped.ops().enumerate() {
        for (step, schema) in steps {
            changes.add(i, before, schema);
            if matches!(op.kind, OperatorKind::DropUnique(..)) {
                changes.drop_uniques(i, before_source, before, schema)?;
            }
            before = schema;
            before_source = &step.after;
        }
    }
    Ok(changes)
}

/// Returns the position of the operator of `stepped`, applied to `before`, whose step last leaves
/// an old key of `pending` without an image it had, or 0 when `before` leaves one without.
fn imaging(before: &Schema, stepped: &Stepped, pending: &[Pending]) -> usize {
    let lost = |s: &Schema| !super::pending::imageless_keys(s, pending).is_empty();
    let mut had = !lost(before);
    let mut at = 0;
    for (i, (_, steps)) in stepped.ops().enumerate() {
        for (_, schema) in steps {
            let has = !lost(schema);
            if had && !has {
                at = i;
            }
            had = has;
        }
    }
    at
}

/// Returns, for each failure of `f`, its operators' then its recheck's, the pending object of
/// `pending` whose kept declaration in the result holds the failure's span, when one does.
fn owners(f: &evolve::Failed, pending: &[Pending]) -> Vec<Option<String>> {
    let evolve::Failed::Refused(r) = f else {
        return Vec::new();
    };
    let files = r
        .result
        .as_ref()
        .and_then(|s| s.files().ok())
        .unwrap_or_default();
    let owner = |d: &FileDiag| {
        let (_, file) = files.iter().find(|(id, _)| *id == d.file)?;
        let within = |s: Span| s.start <= d.diag.span.start && d.diag.span.end <= s.end;
        let (name, check) = file.decls.iter().find_map(|decl| match decl {
            Decl::Constraint(c) if within(c.span) => Some((&c.name.text, false)),
            Decl::Check(c) if within(c.span) => Some((&c.name.text, true)),
            _ => None,
        })?;
        pending
            .iter()
            .find(|p| {
                p.module == file.module.text
                    && p.name == *name
                    && matches!(
                        (&p.kind, check),
                        (Kind::Constraint(_), false) | (Kind::Check(_), true)
                    )
            })
            .map(Pending::object)
    };
    let ops = r.operators.iter().map(|_| None);
    ops.chain(r.recheck.iter().map(owner)).collect()
}

/// Returns `file` without the operators `skipped` names by position.
fn skipping(file: &MigrationFile, skipped: &BTreeSet<usize>) -> MigrationFile {
    MigrationFile {
        module: file.module.clone(),
        operators: file
            .operators
            .iter()
            .enumerate()
            .filter(|(i, _)| !skipped.contains(i))
            .map(|(_, op)| op.clone())
            .collect(),
    }
}

/// Returns the source `source`'s log folds to keeping every object of `pending` from the
/// migration that dropped it, checked, or `None` when nothing is pending: each kept
/// constraint and check put back after the operators that drop it, and each `drop unique` of a
/// kept unique skipped.
///
/// Fail with [`Error::Corrupt`] when a drop names a version the log does not hold or a unique
/// its migration does not drop, and with [`Error::Unaccepted`] when a migration of the log does
/// not apply keeping the drops. Every other failure keeps its class.
pub(super) fn enforcing(source: &Source, pending: &[Pending]) -> Result<Option<Checked>, Error> {
    let Some(first) = pending.iter().map(|p| p.dropped).min() else {
        return Ok(None);
    };
    let Some(start) = source.log().iter().find(|l| l.number == first) else {
        return Err(Rot::DropVersion(first).into());
    };
    let mut at = (*start.before).clone();
    let mut version = first;
    for l in source.log().iter().filter(|l| l.number >= first) {
        version = l.number;
        let refused = |why: Vec<String>| Error::Unaccepted(l.number, why);
        let mut file = parse_migration(&l.text, Options::default())
            .ok()
            .map_err(|ds| refused(ds.into_iter().map(|d| d.message).collect()))?;
        let kept: Vec<Kept> = pending
            .iter()
            .filter(|p| p.dropped <= l.number)
            .filter_map(Pending::kept)
            .collect();
        let held: BTreeSet<CatIndexId> = pending
            .iter()
            .filter(|p| p.dropped == l.number)
            .filter_map(Pending::unique)
            .collect();
        if !held.is_empty() {
            let before = l
                .before
                .schema()
                .map_err(|ds| refused(ds.into_iter().map(|d| d.diag.message).collect()))?;
            let steps = l
                .before
                .record_steps(&file, &l.text)
                .map_err(|r| unaccepted_at(l.number, super::refusal(r, Error::Refused)))?;
            let stepped = super::steps::stepped(&steps, &file, &l.text)
                .map_err(|e| unaccepted_at(l.number, e))?;
            let drops = changes(&l.before, &before, &stepped)
                .map_err(|e| unaccepted_at(l.number, e))?
                .uniques;
            let skipped: BTreeSet<usize> = drops
                .iter()
                .filter(|u| held.contains(&u.id))
                .map(|u| u.op)
                .collect();
            if let Some(id) = held.iter().find(|id| !drops.iter().any(|u| u.id == **id)) {
                return Err(Rot::UniqueNotDropped {
                    index: *id,
                    version: l.number,
                }
                .into());
            }
            file = skipping(&file, &skipped);
        }
        at = at
            .migrate_keeping(&file, &l.text, &kept)
            .map_err(|r| unaccepted_at(l.number, super::unaccepted(l.number, r)))?;
    }
    let enforced = checked(at, |ds| {
        Error::Unchecked("the schema keeping the pending drops".to_string(), ds)
    })
    .map_err(|e| unaccepted_at(version, e))?;
    keeps_ids(enforced.schema(), pending).map_err(|e| unaccepted_at(version, e))?;
    Ok(Some(enforced))
}

/// Returns `e`, met applying the migration of `version` keeping pending drops, with a refusal as
/// that migration being unaccepted.
pub(super) fn unaccepted_at(version: u64, e: Error) -> Error {
    match e {
        Error::Refused(why) => Error::Unaccepted(version, why),
        Error::RefusedBy { operator, why } => Error::Unaccepted(
            version,
            why.iter().map(|w| format!("`{operator}`: {w}")).collect(),
        ),
        e @ (Error::Page(_)
        | Error::Heap(_)
        | Error::Index(_)
        | Error::Txn(_)
        | Error::Missing(_)
        | Error::Corrupt(_)
        | Error::Unaccepted(..)
        | Error::Internal(_)
        | Error::MissingFunction(_)
        | Error::OtherVersion(_)
        | Error::Unchecked(..)
        | Error::PastLimits(_)
        | Error::FormatTooOld(_)
        | Error::Conflict(_)
        | Error::Spill(_)
        | Error::Quarantined(_)) => e,
    }
}

/// Fail with [`Broken::KeptConstraint`] when `schema` holds no constraint or check of the id a
/// constraint or check drop of `pending` holds.
fn keeps_ids(schema: &Schema, pending: &[Pending]) -> Result<(), Error> {
    let ids = schema.ids();
    for p in pending {
        let (id, kept) = match p.kind {
            Kind::Constraint(id) => (id, ids.constraint_ids().any(|(_, k)| k == id)),
            Kind::Check(id) => (id, ids.check_ids().any(|(_, k)| k == id)),
            Kind::Unique(_) | Kind::Key { .. } => continue,
        };
        if !kept {
            return Err(Error::from(Broken::KeptConstraint(p.object(), id)));
        }
    }
    Ok(())
}

/// A schema with its checked program and the texts of its index elements.
pub(super) type Judged<'a> = (&'a Compiled, &'a ElementTexts);

/// The live readers of a catalog, the drops they hold back, and the schema that keeps every
/// held object.
#[derive(Clone, Debug, Default)]
pub(super) struct Readers {
    pub(super) live: BTreeMap<String, Pinned>,
    pub(super) pending: Vec<Pending>,
    /// The source with every pending-dropped object kept, while one is pending.
    enforced: Option<Arc<Checked>>,
}

impl Readers {
    /// Returns the readers `live` holding back the drops of `pending`, whose kept source is
    /// `enforced`.
    pub(super) fn new(
        live: BTreeMap<String, Pinned>,
        pending: Vec<Pending>,
        enforced: Option<Arc<Checked>>,
    ) -> Readers {
        Readers {
            live,
            pending,
            enforced,
        }
    }

    /// Returns these readers holding back the drops of `pending` instead, under the kept
    /// source they have.
    pub(super) fn holding(&self, pending: Vec<Pending>) -> Readers {
        Readers::new(self.live.clone(), pending, self.enforced.clone())
    }

    /// Returns what rows are judged against at `version`: `version`, or, while a drop is
    /// pending, the version that keeps every pending-dropped object.
    pub(super) fn checking<'a>(&'a self, version: Judged<'a>) -> Judged<'a> {
        (self.enforced.as_deref()).map_or(version, |e| (&e.compiled, &e.texts))
    }

    /// Returns the source of what [`Readers::checking`] returns at `source`.
    fn checking_source<'a>(&'a self, source: &'a Source) -> &'a Source {
        (self.enforced.as_deref()).map_or(source, |e| &e.source)
    }
}

impl Catalog {
    /// Record `reader` as live.
    ///
    /// Fail with [`Error::Refused`] when a reader of the same bundle is live, when its version
    /// is not one of the log's, or when a name it reads or writes does not resolve at its
    /// version.
    pub fn register(&mut self, reader: Reader) -> Result<(), Error> {
        let refused = |why: String| Err(Error::Refused(vec![why]));
        if self.readers.live.contains_key(&reader.bundle) {
            return refused(format!("bundle {} is already live", reader.bundle));
        }
        let Some(schema) = schema_at(&self.source, reader.version) else {
            return refused(format!(
                "bundle {} is pinned at version {}, which the log does not hold",
                reader.bundle, reader.version
            ));
        };
        let pinned = pinned(&schema, &reader).map_err(|why| {
            Error::Refused(vec![format!(
                "bundle {} at version {}: {why}",
                reader.bundle, reader.version
            )])
        })?;
        self.readers.live.insert(reader.bundle, pinned);
        Ok(())
    }

    /// Returns the pending drops with the bundles holding each, in the order they applied,
    /// those of one migration by object.
    #[cfg(test)]
    pub fn pending_drops(&self) -> Vec<PendingDrop> {
        let mut out: Vec<PendingDrop> = self
            .readers
            .pending
            .iter()
            .map(|p| PendingDrop {
                object: p.object(),
                dropped: p.dropped,
                blocking: p.blocking.iter().cloned().collect(),
            })
            .collect();
        out.sort_by(|a, b| (a.dropped, &a.object).cmp(&(b.dropped, &b.object)));
        out
    }

    /// Returns the readers after `file`, the migration of `text` whose result is `next` and whose
    /// operators take `stepped`: the live readers, the drops they hold back, and the source that
    /// keeps every pending-dropped object.
    ///
    /// Fail with [`Error::RefusedBy`] when a live reader refuses the migration, naming the
    /// reader, or when the migration leaves a pending-dropped object without an image, naming
    /// the object; each names the first operator whose step it refuses.
    pub(super) fn held(
        &self,
        file: &MigrationFile,
        text: &str,
        next: &Checked,
        stepped: &Stepped,
    ) -> Result<Readers, Error> {
        let changes = changes(&self.source, self.schema(), stepped)?;
        let operators: Vec<String> = stepped
            .ops()
            .map(|(op, _)| operator_text(text, &op))
            .collect();
        let refused_by = |at: usize, why: Vec<String>| Error::RefusedBy {
            operator: operators.get(at).cloned().unwrap_or_default(),
            why,
        };
        let now = &next.schema();
        let mut refused: Vec<(usize, String)> = Vec::new();
        let at = |op: usize| move |why: String| (op, why);
        for (&(rid, cid), &op) in &changes.enabled {
            if let Some((r, c)) = ctor_of(now, rid, cid) {
                refused.extend(self.enabling(now, r, c).into_iter().map(at(op)));
            }
        }
        for (&cid, &op) in &changes.value_enabled {
            if let Some((def, c)) = value_ctor(now, cid) {
                refused.extend(self.enabling_value(now, def, c).into_iter().map(at(op)));
            }
        }
        for (&(rid, cid, verb), &op) in &changes.stopped {
            if let Some((r, c)) = ctor_of(now, rid, cid) {
                refused.extend(self.writing(now, r, c, verb).into_iter().map(at(op)));
            }
        }
        refused.extend(self.uniques_held(now, &changes));
        let dropped: Vec<Pending> = changes
            .dropped
            .into_iter()
            .filter_map(|d| self.dropping(next, d))
            .collect();
        let held = self.readers.pending.iter().chain(&dropped);
        refused.extend(names_held(now, &changes.added, held));
        if let Some(first) = refused.iter().map(|(op, _)| *op).min() {
            return Err(refused_by(
                first,
                refused.into_iter().map(|(_, why)| why).collect(),
            ));
        }
        let mut pending = self.readers.pending.clone();
        let keys = self.relaxed_keys(next, &pending);
        pending.extend(keys);
        let lost = super::pending::imageless_keys(now, &pending);
        if !lost.is_empty() {
            let at = imaging(self.schema(), stepped, &pending);
            return Err(refused_by(at, lost));
        }
        pending.extend(dropped);
        let mut skipped = BTreeSet::new();
        for u in changes.uniques {
            let op = u.op;
            if let Some(p) = self.dropping_unique(next, u) {
                skipped.insert(op);
                pending.push(p);
            }
        }
        let mut readers = Readers {
            live: self.readers.live.clone(),
            pending,
            enforced: None,
        };
        if readers.pending.is_empty() {
            return Ok(readers);
        }
        let base = self.readers.checking_source(&self.source);
        let kept: Vec<Kept> = readers.pending.iter().filter_map(Pending::kept).collect();
        let objects: Vec<String> = readers.pending.iter().map(Pending::object).collect();
        let no_image =
            |why: &str| format!("pending-dropped {} has no image: {why}", objects.join(", "));
        let source = base
            .migrate_keeping(&skipping(file, &skipped), text, &kept)
            .map_err(|r| {
                let evolve::Failed::Refused(refused) = &r else {
                    return super::refusal(r, Error::Refused);
                };
                let kept: Vec<usize> = (0..file.operators.len())
                    .filter(|i| !skipped.contains(i))
                    .collect();
                let at = kept[refused.blamed()];
                let owners = owners(&r, &readers.pending);
                super::refusal(r, |why| {
                    let line = |(i, w): (usize, &String)| match owners.get(i).cloned().flatten() {
                        Some(o) => format!("pending-dropped {o} has no image: {w}"),
                        None => no_image(w),
                    };
                    refused_by(at, why.iter().enumerate().map(line).collect())
                })
            })?;
        let enforced = checked(source, |ds| {
            Error::Refused(ds.iter().map(|d| no_image(&d.message)).collect())
        })?;
        if !enforced
            .schema()
            .ids()
            .relation_ids()
            .eq(now.ids().relation_ids())
        {
            return Err(Error::from(Broken::PendingRelations));
        }
        keeps_ids(enforced.schema(), &readers.pending)?;
        readers.enforced = Some(Arc::new(enforced));
        Ok(readers)
    }

    /// Returns why the live readers refuse enabling constructor `c` of relation `r` in a
    /// migration whose result is `next`.
    fn enabling(&self, next: &Schema, r: RelId, c: u32) -> Vec<String> {
        let rel = next.relation(r);
        let shown = format!(
            "{}.{}",
            rel.name,
            next.ty(rel.elem_def).ctors()[c as usize].name
        );
        self.readers
            .live
            .iter()
            .filter_map(|(bundle, p)| {
                let reads = p.blocks(next, r, c)?;
                Some(blocked(&format!("enable {shown}"), bundle, p, &reads))
            })
            .collect()
    }

    /// Returns why the live readers refuse enabling constructor `c` of type `def` in a migration
    /// whose result is `next`.
    fn enabling_value(&self, next: &Schema, def: TypeId, c: u32) -> Vec<String> {
        let shown = format!(
            "{}.{}",
            next.ty(def).name,
            next.ty(def).ctors()[c as usize].name
        );
        self.readers
            .live
            .iter()
            .filter_map(|(bundle, p)| {
                let reads = p.blocks_value(next, def, c)?;
                Some(blocked(&format!("enable {shown}"), bundle, p, &reads))
            })
            .collect()
    }

    /// Returns why the live readers refuse `verb`, a deprecate or disable of constructor `c` of
    /// relation `r` in a migration whose result is `next`: each bundle that can write into the
    /// constructor relation.
    fn writing(&self, next: &Schema, r: RelId, c: u32, verb: &str) -> Vec<String> {
        let rel = next.relation(r);
        let id = (rel_id(next, r), ctor_id(next, r, c));
        let shown = format!(
            "{}.{}",
            rel.name,
            next.ty(rel.elem_def).ctors()[c as usize].name
        );
        self.readers
            .live
            .iter()
            .filter(|(_, p)| p.writes.contains(&id))
            .map(|(bundle, p)| {
                format!(
                    "`{verb} {shown}` is refused: bundle {bundle}, pinned at version {}, can write into {shown}",
                    p.version
                )
            })
            .collect()
    }

    /// Returns the pending drop of `d` by the migration whose result is `next` when a live
    /// reader holds it.
    fn dropping(&self, next: &Checked, d: Dropped) -> Option<Pending> {
        let blocking: BTreeSet<String> = self
            .readers
            .live
            .iter()
            .filter(|(_, p)| p.constraints.contains(&d.id))
            .map(|(b, _)| b.clone())
            .collect();
        let kind = if d.check {
            Kind::Check(d.id)
        } else {
            Kind::Constraint(d.id)
        };
        (!blocking.is_empty()).then(|| Pending::of(next, (d.module, d.name), kind, blocking))
    }

    /// Returns the pending drop of `u` by the migration whose result is `next` when a live
    /// reader holds it.
    fn dropping_unique(&self, next: &Checked, u: DroppedUnique) -> Option<Pending> {
        let blocking: BTreeSet<String> = self
            .readers
            .live
            .iter()
            .filter(|(_, p)| p.uniques.contains(&u.id))
            .map(|(b, _)| b.clone())
            .collect();
        (!blocking.is_empty())
            .then(|| Pending::of(next, (u.module, u.name), Kind::Unique(u.id), blocking))
    }

    /// Returns why the live readers refuse a migration whose result is `next` for each unique
    /// a reader's version holds that a step of `changes` drops otherwise than by a
    /// `drop unique`, with the position of the operator whose step drops it.
    fn uniques_held(&self, next: &Schema, changes: &Changes) -> Vec<(usize, String)> {
        let kept = unique_ids(next);
        let mut why = Vec::new();
        for (id, (r, clause)) in uniques(self.schema(), &self.texts) {
            let Some(&op) = changes.lost_uniques.get(&id) else {
                continue;
            };
            if kept.contains(&id) || changes.uniques.iter().any(|u| u.id == id) {
                continue;
            }
            for (bundle, p) in self
                .readers
                .live
                .iter()
                .filter(|(_, p)| p.uniques.contains(&id))
            {
                why.push((op, format!(
                    "the drop of `{clause}` in {} is refused: bundle {bundle}, pinned at version {}, \
                     relies on it, and only a `drop unique` holds a unique pending; it applies \
                     once the bundle is released",
                    self.schema().relation(r).name,
                    p.version
                )));
            }
        }
        why
    }

    /// Returns the pending drop of the old key of each relaxing rekey from this catalog to
    /// `applying` that a live reader holds, its index allocated beside those of `pending`.
    fn relaxed_keys(&self, applying: &Checked, pending: &[Pending]) -> Vec<Pending> {
        let next = &applying.schema();
        let mut out: Vec<Pending> = Vec::new();
        for (r, id) in self.schema().ids().relation_ids() {
            let Some(after) = relation(next, id) else {
                continue;
            };
            let rel = self.schema().relation(r);
            let Some(old) = super::pending::key_fields(self.schema(), r, &rel.key) else {
                continue;
            };
            let new = super::pending::key_fields(next, after, &next.relation(after).key);
            if new.is_some_and(|new| new.iter().all(|f| old.contains(f))) {
                continue;
            }
            let blocking: BTreeSet<String> = self
                .readers
                .live
                .iter()
                .filter(|(_, p)| p.relations.contains(&id))
                .map(|(b, _)| b.clone())
                .collect();
            if blocking.is_empty() {
                continue;
            }
            let index = super::pending::held_key_index(&[pending, &out].concat());
            let spec = super::IndexSpec::key(self.schema(), r, index, rel.key.clone());
            let named = (
                self.schema().module(rel.module).name.clone(),
                format!("{} {}", rel.name, spec.clause),
            );
            let kind = Kind::Key {
                index,
                relation: id,
                fields: old,
            };
            out.push(Pending::of(applying, named, kind, blocking));
        }
        out
    }
}

/// Returns the refusal of `op` for `bundle`, pinned as `p`, which `reads` as stated.
fn blocked(op: &str, bundle: &str, p: &Pinned, reads: &str) -> String {
    format!(
        "`{op}` is refused: bundle {bundle}, pinned at version {}, {reads}; a bundle stops \
         blocking only when it is released",
        p.version
    )
}

/// Returns the constructor `reader` would read that is enabled now and absent at its
/// version, as `relation.Ctor` or `Type.Ctor`, under the rules by which a live reader blocks
/// its enable, `source` holding the log and `now` its current schema.
///
/// Fail with [`Error::Refused`] when the reader's version is not one of the log's or a name
/// it reads or writes does not resolve at its version.
pub fn stale(source: &Source, now: &Schema, reader: &Reader) -> Result<Option<String>, Error> {
    let refused = |why: String| Error::Refused(vec![why]);
    let schema = schema_at(source, reader.version).ok_or_else(|| {
        refused(format!(
            "bundle {} is pinned at version {}, which the log does not hold",
            reader.bundle, reader.version
        ))
    })?;
    let p = pinned(&schema, reader).map_err(refused)?;
    for (r, _) in now.ids().relation_ids() {
        let rel = now.relation(r);
        for (c, ctor) in now.ty(rel.elem_def).ctors().iter().enumerate() {
            let c = u32::try_from(c).expect("a sum has fewer than 2^32 constructors");
            if now.stage(r, c) != Stage::Declared && p.blocks(now, r, c).is_some() {
                return Ok(Some(format!("{}.{}", rel.name, ctor.name)));
            }
        }
    }
    for (c, _) in now.ids().ctor_ids() {
        let (def, k) = (c.def(), c.index());
        if now.value_stage(def, k) != Stage::Declared && p.blocks_value(now, def, k).is_some() {
            let t = now.ty(def);
            return Ok(Some(format!("{}.{}", t.name, t.ctors()[k as usize].name)));
        }
    }
    Ok(None)
}

/// Returns the schema of the version `v` of `source`'s log.
fn schema_at(source: &Source, v: u64) -> Option<Schema> {
    source_at(source, v)?.schema().ok()
}

/// Returns the source of the version `v` of `source`'s log.
pub fn source_at(source: &Source, v: u64) -> Option<Source> {
    if v == 0 || v > source.version() {
        return None;
    }
    if v == source.version() {
        return Some(source.clone());
    }
    let after = source.log().iter().find(|l| l.number == v + 1)?;
    Some((*after.before).clone())
}

/// Returns `reader` with its names resolved in `schema`, the schema of its version.
fn pinned(schema: &Schema, reader: &Reader) -> Result<Pinned, String> {
    let mut p = Pinned {
        version: reader.version,
        hides_new: reader.hides_new,
        whole: BTreeSet::new(),
        references: Vec::new(),
        fields: BTreeSet::new(),
        writes: BTreeSet::new(),
        ctors: schema.ids().ctor_ids().map(|(_, id)| id).collect(),
        constraints: schema
            .ids()
            .constraint_ids()
            .map(|(_, id)| id)
            .chain(schema.ids().check_ids().map(|(_, id)| id))
            .collect(),
        uniques: unique_ids(schema),
        relations: schema.ids().relation_ids().map(|(_, id)| id).collect(),
    };
    for read in &reader.reads {
        match read {
            Read::Relation(name) => {
                let r = relation_named(schema, name)?;
                p.whole.insert(rel_id(schema, r));
            }
            Read::Reference { relation, set } => {
                let r = relation_named(schema, relation)?;
                let set = match set {
                    Some(s) => Some(
                        schema
                            .ids()
                            .set_id(r, s)
                            .ok_or_else(|| format!("{relation} has no set {s}"))?,
                    ),
                    None => None,
                };
                p.references.push((rel_id(schema, r), set));
            }
            Read::Field { relation, field } => {
                let r = relation_named(schema, relation)?;
                let td = schema.ty(schema.relation(r).elem_def);
                let before = p.fields.len();
                p.fields.extend(
                    schema
                        .ids()
                        .type_field_ids(td.id())
                        .filter(|(f, _)| td.field_def(*f).name == *field)
                        .map(|(_, id)| id),
                );
                if p.fields.len() == before {
                    return Err(format!("{relation} has no field {field}"));
                }
            }
        }
    }
    for (relation, ctor) in &reader.writes {
        let r = relation_named(schema, relation)?;
        let c = schema
            .ty(schema.relation(r).elem_def)
            .ctor_index(ctor)
            .ok_or_else(|| format!("{relation} has no constructor {ctor}"))?;
        p.writes.insert((rel_id(schema, r), ctor_id(schema, r, c)));
    }
    Ok(p)
}

/// Returns the relation `module::name` of `schema`.
fn relation_named(schema: &Schema, qualified: &str) -> Result<RelId, String> {
    let found = qualified.split_once("::").and_then(|(m, n)| {
        let m = schema.module_named(m)?;
        match schema.module(m).values.get(n) {
            Some(ValueDef::Relation(r)) => Some(*r),
            _ => None,
        }
    });
    found.ok_or_else(|| format!("there is no relation {qualified}"))
}

/// Returns whether named set `set` of relation `r` holds constructor `c`.
fn in_set(schema: &Schema, r: RelId, set: CatSetId, c: u32) -> bool {
    schema
        .relation(r)
        .sets
        .iter()
        .any(|s| schema.ids().set_id(r, &s.name) == Some(set) && s.ctors.contains(&c))
}

/// Returns the relation and constructor index of constructor relation `(rid, cid)` of `schema`.
fn ctor_of(schema: &Schema, rid: CatRelId, cid: CatCtorId) -> Option<(RelId, u32)> {
    let r = relation(schema, rid)?;
    Some((r, ctor_index(schema, r, cid)?))
}

/// Returns the type and constructor index of constructor `cid` of `schema`.
fn value_ctor(schema: &Schema, cid: CatCtorId) -> Option<(TypeId, u32)> {
    schema.ids().ctor_of(cid).map(|c| (c.def(), c.index()))
}

/// Returns the type, in `schema`, of each field `p` reads.
fn read_fields<'s>(schema: &'s Schema, p: &'s Pinned) -> impl Iterator<Item = &'s Ty> {
    p.fields
        .iter()
        .flat_map(|&id| schema.ids().field_of(id))
        .map(|f| &schema.ty(f.def()).field_def(f).ty)
}

/// Returns the catalog id of each declared unique of `schema`, with its relation and its clause,
/// each index element that is not a field path as `texts` holds it.
fn uniques(schema: &Schema, texts: &ElementTexts) -> BTreeMap<CatIndexId, (RelId, String)> {
    let declared = unique_ids(schema);
    super::indexes(schema, texts)
        .into_values()
        .filter(|s| declared.contains(&s.id))
        .map(|s| (s.id, (s.relation, s.clause)))
        .collect()
}

/// Returns the catalog id of each declared unique of `schema`.
pub(super) fn unique_ids(schema: &Schema) -> BTreeSet<CatIndexId> {
    schema
        .ids()
        .index_ids()
        .filter(|&((r, i), _)| schema.relation(r).indexes[i].unique)
        .map(|(_, id)| id)
        .collect()
}

/// Returns why a migration whose result is `next` is refused for each constraint or check it
/// declares under the name of a constraint or check of `pending`, with the position of the
/// operator that `adding` names as adding it.
fn names_held<'p>(
    next: &Schema,
    adding: &BTreeMap<(String, String), usize>,
    pending: impl Iterator<Item = &'p Pending>,
) -> Vec<(usize, String)> {
    pending
        .filter_map(|p| {
            let &op = adding.get(&(p.module.clone(), p.name.clone()))?;
            let held = match p.kind {
                Kind::Constraint(_) => "constraint",
                Kind::Check(_) => "check",
                Kind::Unique(_) | Kind::Key { .. } => return None,
            };
            let named = |m, n: &str| next.module(m).name == p.module && n == p.name;
            let added = if next.constraints.iter().any(|c| named(c.module, &c.name)) {
                "constraint"
            } else if next.checks.iter().any(|c| named(c.module, &c.name)) {
                "check"
            } else {
                return None;
            };
            let holders: Vec<&str> = p.blocking.iter().map(String::as_str).collect();
            Some((
                op,
                format!(
                    "adding {added} {} is refused: pending-dropped {held} {}, held by {}, keeps \
                     its name until its drop takes effect",
                    p.object(),
                    p.object(),
                    holders.join(", ")
                ),
            ))
        })
        .collect()
}

/// Returns `source` checked, the checker's diagnostics refused as `refused` refuses them.
pub(super) fn checked(
    source: Source,
    refused: impl FnOnce(Vec<Diag>) -> Error,
) -> Result<Checked, Error> {
    let compiled = source.check().map_err(|ds| {
        super::past_limits(ds.iter().map(|d| &d.diag))
            .unwrap_or_else(|| refused(ds.into_iter().map(|d| d.diag).collect()))
    })?;
    Checked::new(source, compiled)
}
