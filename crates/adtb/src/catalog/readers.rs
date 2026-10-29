use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

use adtb_lang::ast::{Ident, MigrationFile, Operator, OperatorKind, QName};
use adtb_lang::evolve::{Kept, Source};
use adtb_lang::parse::{Options, parse_migration};
use adtb_lang::schema::{Schema, Stage, ValueDef};
use adtb_lang::span::{Diag, Span};
use adtb_lang::types::{RelId, Ty, TypeId};

use super::ids::{
    CatConstraintId, CatCtorId, CatFieldId, CatIndexId, CatRelId, CatSetId, ctor_id, ctor_index,
    rel_id, relation,
};
use super::{Catalog, Checked, Error, ctor_count};

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
    Constraint,
    Check,
    Unique(CatIndexId),
    /// The old key of a relaxing rekey of `relation`, over `fields`, enforced by index `index`.
    Key {
        index: CatIndexId,
        relation: CatRelId,
        fields: Vec<CatFieldId>,
    },
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
    pub(super) fn object(&self) -> String {
        format!("{}::{}", self.module, self.name)
    }

    /// Returns the declaration a migration keeps for this drop, `None` for a unique or an old key.
    pub(super) fn kept(&self) -> Option<Kept> {
        let check = match self.kind {
            Kind::Constraint => false,
            Kind::Check => true,
            Kind::Unique(_) | Kind::Key { .. } => return None,
        };
        Some(Kept {
            module: self.module.clone(),
            name: self.name.clone(),
            check,
        })
    }

    fn unique(&self) -> Option<CatIndexId> {
        match self.kind {
            Kind::Unique(id) => Some(id),
            Kind::Constraint | Kind::Check | Kind::Key { .. } => None,
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

/// What the steps of a migration do to write states and constraints, by catalog id.
#[derive(Debug, Default)]
struct Changes {
    /// The constructor relations a step makes writable.
    enabled: BTreeSet<(CatRelId, CatCtorId)>,
    /// The value-level constructors a step makes writable.
    value_enabled: BTreeSet<CatCtorId>,
    /// The constructor relations a step stops writes into, with the operator that does so.
    stopped: BTreeSet<(CatRelId, CatCtorId, &'static str)>,
    dropped: Vec<Dropped>,
    uniques: Vec<DroppedUnique>,
}

impl Changes {
    /// Record what the step from `a` to `b` changes.
    fn add(&mut self, a: &Schema, b: &Schema) {
        for &r in b.ids.relations.keys() {
            let rid = rel_id(b, r);
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
                        self.stopped.insert((rid, cid, "deprecate"));
                    }
                    (Stage::Enabled, Stage::Declared) => {
                        self.stopped.insert((rid, cid, "disable"));
                    }
                    (Stage::Declared | Stage::Deprecated, Stage::Enabled) => {
                        self.enabled.insert((rid, cid));
                    }
                    _ => {}
                }
            }
        }
        let was: BTreeMap<u64, (TypeId, u32)> =
            a.ids.ctors.iter().map(|(&at, &id)| (id, at)).collect();
        for (&(def, c), &id) in &b.ids.ctors {
            if let Some(&(d, k)) = was.get(&id)
                && a.value_stage(d, k) != Stage::Enabled
                && b.value_stage(def, c) == Stage::Enabled
            {
                self.value_enabled.insert(CatCtorId(id));
            }
        }
        let kept: BTreeSet<u64> = b
            .ids
            .constraints
            .values()
            .chain(b.ids.checks.values())
            .copied()
            .collect();
        let named = |m, name: &str, id, check| Dropped {
            id: CatConstraintId(id),
            module: a.module(m).name.clone(),
            name: name.to_string(),
            check,
        };
        for (&i, &id) in &a.ids.constraints {
            if !kept.contains(&id) {
                let c = &a.constraints[i];
                self.dropped.push(named(c.module, &c.name, id, false));
            }
        }
        for (&i, &id) in &a.ids.checks {
            if !kept.contains(&id) {
                let c = &a.checks[i];
                self.dropped.push(named(c.module, &c.name, id, true));
            }
        }
    }

    /// Record each unique the step from `a` to `b` of operator `op`, a `drop unique`, drops.
    fn drop_uniques(&mut self, op: usize, a: &Schema, b: &Schema) {
        let kept = uniques(b);
        for (id, (r, clause)) in uniques(a) {
            if kept.contains_key(&id) {
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
    }
}

/// Returns what the steps of `file`, the migration of `text` applied to `source`, whose schema
/// is `schema`, do to write states, constraints and uniques.
fn changes(
    source: &Source,
    schema: &Schema,
    file: &MigrationFile,
    text: &str,
) -> Result<Changes, Error> {
    let module = &file.module.text;
    let mut at = source.clone();
    at.version += 1;
    at.expecting(module, &file.operators);
    let mut before = schema.clone();
    let mut changes = Changes::default();
    for (i, op) in file.operators.iter().enumerate() {
        let steps = at
            .steps(module, op, text)
            .map_err(|ds| Error::Refused(ds.into_iter().map(|d| d.message).collect()))?;
        for step in steps {
            let Some(schema) = super::steps::built(&step, op, text)? else {
                continue;
            };
            changes.add(&before, &schema);
            if matches!(op.kind, OperatorKind::DropUnique(..)) {
                changes.drop_uniques(i, &before, &schema);
            }
            before = schema;
        }
    }
    Ok(changes)
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
/// not apply keeping the drops.
pub(super) fn enforcing(source: &Source, pending: &[Pending]) -> Result<Option<Checked>, Error> {
    let Some(first) = pending.iter().map(|p| p.dropped).min() else {
        return Ok(None);
    };
    let Some(start) = source.log.iter().find(|l| l.number == first) else {
        return Err(Error::Corrupt(format!(
            "a pending drop names version {first}, which the log does not hold"
        )));
    };
    let mut at = (*start.before).clone();
    for l in source.log.iter().filter(|l| l.number >= first) {
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
            let drops = changes(&l.before, &before, &file, &l.text)?.uniques;
            let skipped: BTreeSet<usize> = drops
                .iter()
                .filter(|u| held.contains(&u.id))
                .map(|u| u.op)
                .collect();
            if let Some(id) = held.iter().find(|id| !drops.iter().any(|u| u.id == **id)) {
                return Err(Error::Corrupt(format!(
                    "unique {id} is held as dropped at version {}, which does not drop it",
                    l.number
                )));
            }
            file = skipping(&file, &skipped);
        }
        at = at
            .migrate_keeping(&file, &l.text, &kept)
            .map_err(|r| refused(super::reasons(r)))?;
    }
    let enforced = checked(at)
        .map_err(|ds| Error::Unchecked("the schema keeping the pending drops".to_string(), ds))?;
    Ok(Some(enforced))
}

/// The live readers of a catalog, the drops they hold back, and the schema that keeps every
/// held object.
#[derive(Clone, Debug, Default)]
pub(super) struct Readers {
    pub(super) live: BTreeMap<String, Pinned>,
    pub(super) pending: Vec<Pending>,
    /// The source with every pending-dropped object kept, while one is pending.
    pub(super) enforced: Option<Rc<Checked>>,
}

impl Readers {
    pub(super) fn enforced(&self) -> Option<&Checked> {
        self.enforced.as_deref()
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

    /// Returns the source of version `v` of the log, `None` when the log holds no version `v`.
    pub fn source_at(&self, v: u64) -> Option<Source> {
        source_at(&self.source, v)
    }

    /// Returns the constructor `reader` would read that is enabled now and absent at its
    /// version, as `relation.Ctor` or `Type.Ctor`, under the rules by which a live reader blocks
    /// its enable.
    ///
    /// Fail with [`Error::Refused`] when the reader's version is not one of the log's or a name
    /// it reads or writes does not resolve at its version.
    pub fn stale(&self, reader: &Reader) -> Result<Option<String>, Error> {
        let refused = |why: String| Error::Refused(vec![why]);
        let schema = schema_at(&self.source, reader.version).ok_or_else(|| {
            refused(format!(
                "bundle {} is pinned at version {}, which the log does not hold",
                reader.bundle, reader.version
            ))
        })?;
        let p = pinned(&schema, reader).map_err(refused)?;
        let now = &self.schema;
        if !p.hides_new {
            for &r in now.ids.relations.keys() {
                let rid = rel_id(now, r);
                let rel = now.relation(r);
                for (c, ctor) in now.ty(rel.elem_def).ctors().iter().enumerate() {
                    let c = u32::try_from(c).expect("a sum has fewer than 2^32 constructors");
                    let cid = ctor_id(now, r, c);
                    if p.ctors.contains(&cid) || now.stage(r, c) == Stage::Declared {
                        continue;
                    }
                    let follows = p
                        .references
                        .iter()
                        .any(|&(to, set)| to == rid && set.is_none_or(|s| in_set(now, r, s, c)));
                    if p.whole.contains(&rid) || follows {
                        return Ok(Some(format!("{}.{}", rel.name, ctor.name)));
                    }
                }
            }
        }
        for (&(def, c), &cid) in &now.ids.ctors {
            if p.ctors.contains(&CatCtorId(cid)) || now.value_stage(def, c) == Stage::Declared {
                continue;
            }
            if read_fields(now, &p).any(|t| now.reaches(t, def)) {
                let t = now.ty(def);
                return Ok(Some(format!("{}.{}", t.name, t.ctors()[c as usize].name)));
            }
        }
        Ok(None)
    }

    /// Returns the pending drops with the bundles holding each, in the order they applied,
    /// those of one migration by object.
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

    /// Returns the readers after `file`, the migration of `text` whose result is `next`: the
    /// live readers, the drops they hold back, and the source that keeps every pending-dropped
    /// object.
    ///
    /// Fail with [`Error::Refused`] when a live reader refuses the migration, naming the
    /// reader, or when the migration leaves a pending-dropped object without an image, naming
    /// the object.
    pub(super) fn held(
        &self,
        file: &MigrationFile,
        text: &str,
        next: &Checked,
    ) -> Result<Readers, Error> {
        let changes = changes(&self.source, &self.schema, file, text)?;
        let now = &next.schema;
        let mut refused = Vec::new();
        for &(rid, cid) in &changes.enabled {
            if let Some((r, c)) = ctor_of(now, rid, cid) {
                refused.extend(self.enabling(now, r, c));
            }
        }
        for &cid in &changes.value_enabled {
            if let Some((def, c)) = value_ctor(now, cid) {
                refused.extend(self.enabling_value(now, def, c));
            }
        }
        for &(rid, cid, verb) in &changes.stopped {
            if let Some((r, c)) = ctor_of(now, rid, cid) {
                refused.extend(self.writing(now, r, c, verb));
            }
        }
        refused.extend(self.uniques_held(now, &changes.uniques));
        if !refused.is_empty() {
            return Err(Error::Refused(refused));
        }
        let (redeclared, mut pending): (Vec<Pending>, Vec<Pending>) = self
            .readers
            .pending
            .iter()
            .cloned()
            .partition(|p| declares(now, p));
        let keys = self.relaxed_keys(now, &pending);
        pending.extend(keys);
        let lost = super::pending::imageless_keys(now, &pending);
        if !lost.is_empty() {
            return Err(Error::Refused(lost));
        }
        pending.extend(changes.dropped.into_iter().filter_map(|d| self.dropping(d)));
        let mut skipped = BTreeSet::new();
        for u in changes.uniques {
            let op = u.op;
            if let Some(p) = self.dropping_unique(u) {
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
        let base = match self.readers.enforced() {
            Some(e) => without(&e.source, &redeclared)?,
            None => self.source.clone(),
        };
        let kept: Vec<Kept> = readers.pending.iter().filter_map(Pending::kept).collect();
        let objects: Vec<String> = readers.pending.iter().map(Pending::object).collect();
        let no_image =
            |why: &str| format!("pending-dropped {} has no image: {why}", objects.join(", "));
        let source = base
            .migrate_keeping(&skipping(file, &skipped), text, &kept)
            .map_err(|r| Error::Refused(super::reasons(r).iter().map(|w| no_image(w)).collect()))?;
        let enforced = checked(source)
            .map_err(|ds| Error::Refused(ds.iter().map(|d| no_image(&d.message)).collect()))?;
        if enforced.schema.ids.relations != now.ids.relations {
            return Err(Error::Internal(
                "the schema keeping pending drops has the relations of the version".to_string(),
            ));
        }
        readers.enforced = Some(Rc::new(enforced));
        Ok(readers)
    }

    /// Returns why the live readers refuse enabling constructor `c` of relation `r` in a
    /// migration whose result is `next`.
    fn enabling(&self, next: &Schema, r: RelId, c: u32) -> Vec<String> {
        let rid = rel_id(next, r);
        let rel = next.relation(r);
        let cid = ctor_id(next, r, c);
        let shown = format!(
            "{}.{}",
            rel.name,
            next.ty(rel.elem_def).ctors()[c as usize].name
        );
        let mut why = Vec::new();
        for (bundle, p) in self.blocking(cid) {
            let reads = if p.whole.contains(&rid) {
                format!("reads {} whole", rel.name)
            } else if p
                .references
                .iter()
                .any(|&(to, set)| to == rid && set.is_none_or(|s| in_set(next, r, s, c)))
            {
                format!("follows a reference that admits {shown}")
            } else {
                continue;
            };
            why.push(blocked(&format!("enable {shown}"), bundle, p, &reads));
        }
        why
    }

    /// Returns why the live readers refuse enabling constructor `c` of type `def` in a migration
    /// whose result is `next`.
    fn enabling_value(&self, next: &Schema, def: TypeId, c: u32) -> Vec<String> {
        let cid = CatCtorId(next.ids.ctors[&(def, c)]);
        let ty = &next.ty(def).name;
        let shown = format!("{ty}.{}", next.ty(def).ctors()[c as usize].name);
        let mut why = Vec::new();
        for (bundle, p) in self
            .readers
            .live
            .iter()
            .filter(|(_, p)| !p.ctors.contains(&cid))
        {
            if read_fields(next, p).any(|t| next.reaches(t, def)) {
                let reads = format!("reads a field whose type reaches {ty}");
                why.push(blocked(&format!("enable {shown}"), bundle, p, &reads));
            }
        }
        why
    }

    /// Returns the live readers that can be blocked by constructor `cid`: those pinned at a
    /// version without it that do not hide new constructors.
    fn blocking(&self, cid: CatCtorId) -> impl Iterator<Item = (&String, &Pinned)> {
        self.readers
            .live
            .iter()
            .filter(move |(_, p)| !p.hides_new && !p.ctors.contains(&cid))
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

    /// Returns the pending drop of `d` when a live reader holds it.
    fn dropping(&self, d: Dropped) -> Option<Pending> {
        let blocking: BTreeSet<String> = self
            .readers
            .live
            .iter()
            .filter(|(_, p)| p.constraints.contains(&d.id))
            .map(|(b, _)| b.clone())
            .collect();
        let kind = if d.check {
            Kind::Check
        } else {
            Kind::Constraint
        };
        (!blocking.is_empty()).then(|| Pending {
            module: d.module,
            name: d.name,
            kind,
            dropped: self.source.version + 1,
            blocking,
        })
    }

    /// Returns the pending drop of `u` when a live reader holds it.
    fn dropping_unique(&self, u: DroppedUnique) -> Option<Pending> {
        let blocking: BTreeSet<String> = self
            .readers
            .live
            .iter()
            .filter(|(_, p)| p.uniques.contains(&u.id))
            .map(|(b, _)| b.clone())
            .collect();
        (!blocking.is_empty()).then(|| Pending {
            module: u.module,
            name: u.name,
            kind: Kind::Unique(u.id),
            dropped: self.source.version + 1,
            blocking,
        })
    }

    /// Returns why the live readers refuse a migration whose result is `next` for each unique
    /// a reader's version holds that it drops otherwise than by a `drop unique` of `dropped`.
    fn uniques_held(&self, next: &Schema, dropped: &[DroppedUnique]) -> Vec<String> {
        let kept = uniques(next);
        let mut why = Vec::new();
        for (id, (r, clause)) in uniques(&self.schema) {
            if kept.contains_key(&id) || dropped.iter().any(|u| u.id == id) {
                continue;
            }
            for (bundle, p) in self
                .readers
                .live
                .iter()
                .filter(|(_, p)| p.uniques.contains(&id))
            {
                why.push(format!(
                    "the drop of `{clause}` in {} is refused: bundle {bundle}, pinned at version {}, \
                     relies on it, and only a `drop unique` holds a unique pending; it applies \
                     once the bundle is released",
                    self.schema.relation(r).name,
                    p.version
                ));
            }
        }
        why
    }

    /// Returns the pending drop of the old key of each relaxing rekey from this catalog to
    /// `next` that a live reader holds, its index allocated beside those of `pending`.
    fn relaxed_keys(&self, next: &Schema, pending: &[Pending]) -> Vec<Pending> {
        let mut out: Vec<Pending> = Vec::new();
        for &r in self.schema.ids.relations.keys() {
            let id = rel_id(&self.schema, r);
            let Some(after) = relation(next, id) else {
                continue;
            };
            let rel = self.schema.relation(r);
            let Some(old) = super::pending::key_fields(&self.schema, r, &rel.key) else {
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
            let spec = super::IndexSpec::key(&self.schema, r, index.0, rel.key.clone());
            out.push(Pending {
                module: self.schema.module(rel.module).name.clone(),
                name: format!("{} {}", rel.name, spec.clause),
                kind: Kind::Key {
                    index,
                    relation: id,
                    fields: old,
                },
                dropped: self.source.version + 1,
                blocking,
            });
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

/// Returns the schema of the version `v` of `source`'s log.
fn schema_at(source: &Source, v: u64) -> Option<Schema> {
    source_at(source, v)?.schema().ok()
}

/// Returns the source of the version `v` of `source`'s log.
fn source_at(source: &Source, v: u64) -> Option<Source> {
    if v == 0 || v > source.version {
        return None;
    }
    if v == source.version {
        return Some(source.clone());
    }
    let after = source.log.iter().find(|l| l.number == v + 1)?;
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
        ctors: schema.ids.ctors.values().map(|&id| CatCtorId(id)).collect(),
        constraints: schema
            .ids
            .constraints
            .values()
            .chain(schema.ids.checks.values())
            .map(|&id| CatConstraintId(id))
            .collect(),
        uniques: uniques(schema).into_keys().collect(),
        relations: schema
            .ids
            .relations
            .keys()
            .map(|&r| rel_id(schema, r))
            .collect(),
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
                    Some(s) => Some(CatSetId(
                        *schema
                            .ids
                            .sets
                            .get(&(r, s.clone()))
                            .ok_or_else(|| format!("{relation} has no set {s}"))?,
                    )),
                    None => None,
                };
                p.references.push((rel_id(schema, r), set));
            }
            Read::Field { relation, field } => {
                let r = relation_named(schema, relation)?;
                let def = schema.relation(r).elem_def;
                let before = p.fields.len();
                p.fields.extend(
                    schema
                        .ids
                        .fields
                        .iter()
                        .filter(|((d, _, n), _)| *d == def && n == field)
                        .map(|(_, &id)| CatFieldId(id)),
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
        .any(|s| schema.ids.sets.get(&(r, s.name.clone())) == Some(&set.0) && s.ctors.contains(&c))
}

/// Returns the relation and constructor index of constructor relation `(rid, cid)` of `schema`.
fn ctor_of(schema: &Schema, rid: CatRelId, cid: CatCtorId) -> Option<(RelId, u32)> {
    let r = relation(schema, rid)?;
    Some((r, ctor_index(schema, r, cid)?))
}

/// Returns the type and constructor index of constructor `cid` of `schema`.
fn value_ctor(schema: &Schema, cid: CatCtorId) -> Option<(TypeId, u32)> {
    schema
        .ids
        .ctors
        .iter()
        .find(|&(_, &id)| id == cid.0)
        .map(|(&at, _)| at)
}

/// Returns the type, in `schema`, of each field `p` reads.
fn read_fields<'s>(schema: &'s Schema, p: &'s Pinned) -> impl Iterator<Item = &'s Ty> {
    schema
        .ids
        .fields
        .iter()
        .filter(move |&(_, &id)| p.fields.contains(&CatFieldId(id)))
        .filter_map(|((def, c, name), _)| field_type(schema, *def, *c, name))
}

/// Returns the type of field `name` of constructor `c` of type `def`, of its header for `None`.
fn field_type<'s>(schema: &'s Schema, def: TypeId, c: Option<u32>, name: &str) -> Option<&'s Ty> {
    let t = schema.ty(def);
    let fields = match c {
        None => t.header(),
        Some(c) => &t.ctors().get(c as usize)?.fields,
    };
    fields.iter().find(|f| f.name == name).map(|f| &f.ty)
}

/// Returns the catalog id of each declared unique of `schema`, with its relation and its clause.
fn uniques(schema: &Schema) -> BTreeMap<CatIndexId, (RelId, String)> {
    let declared: BTreeSet<u64> = schema.ids.indexes.values().copied().collect();
    super::indexes(schema)
        .into_values()
        .filter(|s| s.unique && declared.contains(&s.id))
        .map(|s| (CatIndexId(s.id), (s.relation, s.clause)))
        .collect()
}

/// Returns whether `schema` declares the object of pending drop `p` again.
fn declares(schema: &Schema, p: &Pending) -> bool {
    schema
        .module_named(&p.module)
        .is_some_and(|m| match &p.kind {
            Kind::Check => schema
                .checks
                .iter()
                .any(|c| c.module == m && c.name == p.name),
            Kind::Constraint => schema
                .constraints
                .iter()
                .any(|c| c.module == m && c.name == p.name),
            Kind::Unique(_) | Kind::Key { .. } => false,
        })
}

/// Returns `source` with the objects of `drops` dropped, at its own version and log.
///
/// Fail with [`Error::Unchecked`] when a drop does not apply, and with [`Error::Internal`] for a
/// unique or an old key.
pub(super) fn without(source: &Source, drops: &[Pending]) -> Result<Source, Error> {
    let mut next = source.clone();
    for p in drops {
        let ident = |text: &str| Ident {
            text: text.to_string(),
            span: Span::default(),
        };
        let name = QName {
            module: Some(ident(&p.module)),
            name: ident(&p.name),
        };
        let kind = match &p.kind {
            Kind::Check => OperatorKind::DropCheck(name),
            Kind::Constraint => OperatorKind::DropConstraint(name),
            Kind::Unique(_) | Kind::Key { .. } => {
                return Err(Error::Internal(format!(
                    "pending-dropped {} is never declared again",
                    p.object()
                )));
            }
        };
        let op = Operator {
            kind,
            span: Span::default(),
        };
        next.apply(&p.module, &op, "")
            .map_err(|ds| Error::Unchecked(format!("the drop of {}", p.object()), ds))?;
    }
    Ok(next)
}

/// Returns `source` checked, or the checker's diagnostics.
pub(super) fn checked(source: Source) -> Result<Checked, Vec<Diag>> {
    let (schema, program) = source
        .check()
        .map_err(|ds| ds.into_iter().map(|d| d.diag).collect::<Vec<_>>())?;
    Ok(Checked {
        source,
        schema,
        program: Arc::new(program),
    })
}
