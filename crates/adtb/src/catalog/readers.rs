use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

use adtb_lang::ast::{CtorTarget, MigrationFile, OperatorKind, QName};
use adtb_lang::evolve::Source;
use adtb_lang::parse::{Options, parse_migration};
use adtb_lang::schema::{Schema, Stage, TypeDefKind, ValueDef};
use adtb_lang::types::{RelId, Ty, TypeId};

use super::{Catalog, Checked, Error, targeted};

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
    /// The qualified name of the dropped constraint or check.
    pub object: String,
    /// The version the drop produced.
    pub dropped: u64,
    /// The bundles holding the drop back, in order.
    pub blocking: Vec<String>,
}

/// A reader with its names resolved to catalog ids.
#[derive(Clone, Debug)]
struct Pinned {
    version: u64,
    hides_new: bool,
    whole: BTreeSet<u64>,
    references: Vec<(u64, Option<u64>)>,
    fields: BTreeSet<u64>,
    writes: BTreeSet<(u64, u64)>,
    /// The constructors that exist at `version`.
    ctors: BTreeSet<u64>,
    /// The constraints and checks that exist at `version`.
    constraints: BTreeSet<u64>,
    /// The uniques that exist at `version`.
    uniques: BTreeSet<u64>,
    /// The relations that exist at `version`.
    relations: BTreeSet<u64>,
}

#[derive(Clone, Debug)]
struct Pending {
    module: String,
    name: String,
    check: bool,
    dropped: u64,
    blocking: BTreeSet<String>,
}

impl Pending {
    fn object(&self) -> String {
        format!("{}::{}", self.module, self.name)
    }
}

/// The live readers of a catalog, the drops they hold back, and the schema that keeps every
/// held object.
#[derive(Clone, Debug, Default)]
pub(super) struct Readers {
    live: BTreeMap<String, Pinned>,
    pending: Vec<Pending>,
    /// The source with every pending-dropped object kept, while one is pending.
    enforced: Option<Rc<Checked>>,
}

impl Readers {
    pub(super) fn enforced(&self) -> Option<&Checked> {
        self.enforced.as_deref()
    }
}

impl Catalog {
    /// Record `reader` as live.
    ///
    /// Fails with [`Error::Refused`] when a reader of the same bundle is live, when its version
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

    /// Release the live reader of `bundle`. Every pending drop it was the last to hold takes
    /// effect.
    ///
    /// Fails with [`Error::Refused`] when no reader of `bundle` is live, and with
    /// [`Error::Internal`] when the schema without the drops that take effect does not check.
    pub fn release(&mut self, bundle: &str) -> Result<(), Error> {
        if self.readers.live.remove(bundle).is_none() {
            return Err(Error::Refused(vec![format!("bundle {bundle} is not live")]));
        }
        let mut done = Vec::new();
        for p in &mut self.readers.pending {
            p.blocking.remove(bundle);
        }
        self.readers.pending.retain(|p| {
            let held = !p.blocking.is_empty();
            if !held {
                done.push(p.clone());
            }
            held
        });
        if self.readers.pending.is_empty() {
            self.readers.enforced = None;
        } else if let Some(enforced) = &self.readers.enforced {
            let source = without(&enforced.source, &done).map_err(Error::Internal)?;
            self.readers.enforced = Some(Rc::new(checked(source).map_err(Error::Internal)?));
        }
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
    /// Fails with [`Error::Refused`] when the reader's version is not one of the log's or a name
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
            for (&r, &rid) in &now.ids.relations {
                let rel = now.relation(r);
                for (c, ctor) in now.ty(rel.elem_def).ctors().iter().enumerate() {
                    let c = u32::try_from(c).expect("a sum has fewer than 2^32 constructors");
                    let cid = now.ids.ctors[&(rel.elem_def, c)];
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
        for (&(def, c), cid) in &now.ids.ctors {
            if p.ctors.contains(cid) || now.value_stage(def, c) == Stage::Declared {
                continue;
            }
            if read_fields(now, &p).any(|t| reaches(now, t, def, &mut BTreeSet::new())) {
                let t = now.ty(def);
                return Ok(Some(format!("{}.{}", t.name, t.ctors()[c as usize].name)));
            }
        }
        Ok(None)
    }

    /// Returns the drops live readers hold back, in the order they applied.
    pub fn pending_drops(&self) -> Vec<PendingDrop> {
        self.readers
            .pending
            .iter()
            .map(|p| PendingDrop {
                object: p.object(),
                dropped: p.dropped,
                blocking: p.blocking.iter().cloned().collect(),
            })
            .collect()
    }

    /// Returns the readers after `file`, the migration of `text` whose result is `next`: the
    /// live readers, the drops they hold back, and the source that keeps every pending-dropped
    /// object.
    ///
    /// Fails with [`Error::Refused`] when a live reader refuses the migration, naming the
    /// reader, or when the migration leaves a pending-dropped object without an image, naming
    /// the object.
    pub(super) fn held(
        &self,
        file: &MigrationFile,
        text: &str,
        next: &Checked,
    ) -> Result<Readers, Error> {
        let module = file.module.text.as_str();
        let mut refused = Vec::new();
        let mut held = Vec::new();
        for op in &file.operators {
            match &op.kind {
                OperatorKind::Enable(t) => refused.extend(self.enabling(&next.schema, module, t)),
                OperatorKind::Deprecate(t) => {
                    refused.extend(self.writing(&next.schema, module, t, "deprecate"));
                }
                OperatorKind::Disable(t) => {
                    refused.extend(self.writing(&next.schema, module, t, "disable"));
                }
                OperatorKind::DropConstraint(q) => held.extend(self.dropping(module, q, false)),
                OperatorKind::DropCheck(q) => held.extend(self.dropping(module, q, true)),
                _ => {}
            }
        }
        refused.extend(self.uniques_held(&next.schema));
        refused.extend(self.rekeys_held(&next.schema));
        if !refused.is_empty() {
            return Err(Error::Refused(refused));
        }
        let (redeclared, mut pending): (Vec<Pending>, Vec<Pending>) = self
            .readers
            .pending
            .iter()
            .cloned()
            .partition(|p| declares(&next.schema, p));
        pending.extend(held.iter().cloned());
        let mut readers = Readers {
            live: self.readers.live.clone(),
            pending,
            enforced: None,
        };
        if readers.pending.is_empty() {
            return Ok(readers);
        }
        let base = match self.readers.enforced() {
            Some(e) => without(&e.source, &redeclared).map_err(Error::Internal)?,
            None => self.source.clone(),
        };
        let mut filtered = file.clone();
        filtered
            .operators
            .retain(|op| !drops_any(module, &op.kind, &held));
        let objects: Vec<String> = readers.pending.iter().map(Pending::object).collect();
        let source = base.migrate(&filtered, text).map_err(|r| {
            Error::Refused(
                super::reasons(r)
                    .into_iter()
                    .map(|w| format!("pending-dropped {} has no image: {w}", objects.join(", ")))
                    .collect(),
            )
        })?;
        let enforced = checked(source).map_err(|why| {
            Error::Refused(vec![format!(
                "pending-dropped {} has no image: {why}",
                objects.join(", ")
            )])
        })?;
        if enforced.schema.ids.relations != next.schema.ids.relations {
            return Err(Error::Internal(
                "the schema keeping pending drops has the relations of the version".to_string(),
            ));
        }
        readers.enforced = Some(Rc::new(enforced));
        Ok(readers)
    }

    /// Returns why the live readers refuse `enable target` in a migration of `module` whose
    /// result is `next`.
    fn enabling(&self, next: &Schema, module: &str, target: &CtorTarget) -> Vec<String> {
        let mut why = Vec::new();
        for (r, c) in targeted(next, module, target) {
            let rid = next.ids.relations[&r];
            let rel = next.relation(r);
            let cid = next.ids.ctors[&(rel.elem_def, c)];
            let shown = format!(
                "{}.{}",
                rel.name,
                next.ty(rel.elem_def).ctors()[c as usize].name
            );
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
        }
        if let CtorTarget::Type(path) = target
            && let Some(def) = type_named(next, module, &path.ty)
            && let Some(c) = next.ty(def).ctor_index(&path.ctor.text)
        {
            let cid = next.ids.ctors[&(def, c)];
            let ty = &next.ty(def).name;
            let shown = format!("{ty}.{}", path.ctor.text);
            for (bundle, p) in self
                .readers
                .live
                .iter()
                .filter(|(_, p)| !p.ctors.contains(&cid))
            {
                if read_fields(next, p).any(|t| reaches(next, t, def, &mut BTreeSet::new())) {
                    let reads = format!("reads a field whose type reaches {ty}");
                    why.push(blocked(&format!("enable {shown}"), bundle, p, &reads));
                }
            }
        }
        why
    }

    /// Returns the live readers that can be blocked by constructor `cid`: those pinned at a
    /// version without it that do not hide new constructors.
    fn blocking(&self, cid: u64) -> impl Iterator<Item = (&String, &Pinned)> {
        self.readers
            .live
            .iter()
            .filter(move |(_, p)| !p.hides_new && !p.ctors.contains(&cid))
    }

    /// Returns why the live readers refuse `verb target`, a deprecate or disable in a migration
    /// of `module` whose result is `next`: each bundle that can write into the constructor
    /// relation.
    fn writing(&self, next: &Schema, module: &str, target: &CtorTarget, verb: &str) -> Vec<String> {
        let mut why = Vec::new();
        for (r, c) in targeted(next, module, target) {
            let rel = next.relation(r);
            let id = (next.ids.relations[&r], next.ids.ctors[&(rel.elem_def, c)]);
            let shown = format!(
                "{}.{}",
                rel.name,
                next.ty(rel.elem_def).ctors()[c as usize].name
            );
            for (bundle, p) in &self.readers.live {
                if p.writes.contains(&id) {
                    why.push(format!(
                        "`{verb} {shown}` is refused: bundle {bundle}, pinned at version {}, can write into {shown}",
                        p.version
                    ));
                }
            }
        }
        why
    }

    /// Returns the pending drop of the constraint, or check when `check`, that `q` names in a
    /// migration of `module`, when a live reader holds it.
    fn dropping(&self, module: &str, q: &QName, check: bool) -> Option<Pending> {
        let m = q.module.as_ref().map_or(module, |m| m.text.as_str());
        let id = constraint_id(&self.schema, m, &q.name.text, check)?;
        let blocking: BTreeSet<String> = self
            .readers
            .live
            .iter()
            .filter(|(_, p)| p.constraints.contains(&id))
            .map(|(b, _)| b.clone())
            .collect();
        (!blocking.is_empty()).then(|| Pending {
            module: m.to_string(),
            name: q.name.text.clone(),
            check,
            dropped: self.source.version + 1,
            blocking,
        })
    }

    /// Returns why the live readers refuse a migration whose result is `next` for each unique it
    /// drops that a reader's version holds: this engine holds no dropped unique pending.
    fn uniques_held(&self, next: &Schema) -> Vec<String> {
        let kept = uniques(next);
        let mut why = Vec::new();
        for (id, (r, clause)) in uniques(&self.schema) {
            if kept.contains_key(&id) {
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
                     relies on it, and a dropped unique is not held pending; it applies once the \
                     bundle is released",
                    self.schema.relation(r).name,
                    p.version
                ));
            }
        }
        why
    }

    /// Returns why the live readers refuse a migration whose result is `next` for each relaxing
    /// rekey of a relation a reader's version holds: this engine holds no old key pending.
    fn rekeys_held(&self, next: &Schema) -> Vec<String> {
        let mut why = Vec::new();
        for (&r, &id) in &self.schema.ids.relations {
            let Some(after) = super::relation(next, super::CatRelId(id)) else {
                continue;
            };
            let old = &self.schema.relation(r).key;
            let new = &next.relation(after).key;
            if new.iter().all(|f| old.contains(f)) {
                continue;
            }
            for (bundle, p) in self
                .readers
                .live
                .iter()
                .filter(|(_, p)| p.relations.contains(&id))
            {
                why.push(format!(
                    "the rekey of {} is refused: bundle {bundle}, pinned at version {}, relies on \
                     its old key, and an old key is not held pending; it applies once the bundle \
                     is released",
                    next.relation(after).name,
                    p.version
                ));
            }
        }
        why
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
        ctors: schema.ids.ctors.values().copied().collect(),
        constraints: schema
            .ids
            .constraints
            .values()
            .chain(schema.ids.checks.values())
            .copied()
            .collect(),
        uniques: uniques(schema).into_keys().collect(),
        relations: schema.ids.relations.values().copied().collect(),
    };
    for read in &reader.reads {
        match read {
            Read::Relation(name) => {
                let r = relation_named(schema, name)?;
                p.whole.insert(schema.ids.relations[&r]);
            }
            Read::Reference { relation, set } => {
                let r = relation_named(schema, relation)?;
                let set = match set {
                    Some(s) => Some(
                        *schema
                            .ids
                            .sets
                            .get(&(r, s.clone()))
                            .ok_or_else(|| format!("{relation} has no set {s}"))?,
                    ),
                    None => None,
                };
                p.references.push((schema.ids.relations[&r], set));
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
                        .map(|(_, &id)| id),
                );
                if p.fields.len() == before {
                    return Err(format!("{relation} has no field {field}"));
                }
            }
        }
    }
    for (relation, ctor) in &reader.writes {
        let r = relation_named(schema, relation)?;
        let def = schema.relation(r).elem_def;
        let c = schema
            .ty(def)
            .ctor_index(ctor)
            .ok_or_else(|| format!("{relation} has no constructor {ctor}"))?;
        p.writes
            .insert((schema.ids.relations[&r], schema.ids.ctors[&(def, c)]));
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

/// Returns the type `q` names in a migration of `module`.
pub(super) fn type_named(schema: &Schema, module: &str, q: &QName) -> Option<TypeId> {
    let m = q.module.as_ref().map_or(module, |m| m.text.as_str());
    let m = schema.module_named(m)?;
    schema.module(m).types.get(&q.name.text).copied()
}

/// Returns whether named set `set` of relation `r` holds constructor `c`.
fn in_set(schema: &Schema, r: RelId, set: u64, c: u32) -> bool {
    schema
        .relation(r)
        .sets
        .iter()
        .any(|s| schema.ids.sets.get(&(r, s.name.clone())) == Some(&set) && s.ctors.contains(&c))
}

/// Returns the type, in `schema`, of each field `p` reads.
fn read_fields<'s>(schema: &'s Schema, p: &'s Pinned) -> impl Iterator<Item = &'s Ty> {
    schema
        .ids
        .fields
        .iter()
        .filter(move |(_, id)| p.fields.contains(id))
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

/// Returns whether a value of type `ty` can hold a value of sum `def`: through its type
/// arguments, its structure, or the fields of the types it names.
pub(super) fn reaches(schema: &Schema, ty: &Ty, def: TypeId, seen: &mut BTreeSet<TypeId>) -> bool {
    match ty {
        Ty::Data { def: d, args, .. } => {
            if *d == def {
                return true;
            }
            if args.iter().any(|a| reaches(schema, a, def, seen)) {
                return true;
            }
            if !seen.insert(*d) {
                return false;
            }
            let t = schema.ty(*d);
            match &t.kind {
                TypeDefKind::Sum { header, ctors, .. } => header
                    .iter()
                    .chain(ctors.iter().flat_map(|c| &c.fields))
                    .any(|f| reaches(schema, &f.ty, def, seen)),
                TypeDefKind::Newtype { base, .. } => reaches(schema, base, def, seen),
            }
        }
        Ty::Record(fs) => fs.iter().any(|(_, t)| reaches(schema, t, def, seen)),
        Ty::Sum { ctors, .. } => ctors
            .iter()
            .any(|(_, fs)| fs.iter().any(|(_, t)| reaches(schema, t, def, seen))),
        Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => reaches(schema, t, def, seen),
        _ => false,
    }
}

/// Returns the catalog id of constraint `name`, or check when `check`, of module `module`.
fn constraint_id(schema: &Schema, module: &str, name: &str, check: bool) -> Option<u64> {
    let m = schema.module_named(module)?;
    if check {
        let i = schema
            .checks
            .iter()
            .position(|c| c.module == m && c.name == name)?;
        schema.ids.checks.get(&i).copied()
    } else {
        let i = schema
            .constraints
            .iter()
            .position(|c| c.module == m && c.name == name)?;
        schema.ids.constraints.get(&i).copied()
    }
}

/// Returns the catalog id of each unique of `schema`, with its relation and its clause.
fn uniques(schema: &Schema) -> BTreeMap<u64, (RelId, String)> {
    schema
        .ids
        .indexes
        .iter()
        .filter_map(|(&(r, i), &id)| {
            let def = schema.relation(r).indexes.get(i).filter(|d| d.unique)?;
            let fields: Vec<String> = def
                .fields
                .iter()
                .map(|e| super::field_path(e).unwrap_or_else(|| "..".to_string()))
                .collect();
            Some((id, (r, format!("unique ({})", fields.join(", ")))))
        })
        .collect()
}

/// Returns whether `schema` declares the object of pending drop `p` again.
fn declares(schema: &Schema, p: &Pending) -> bool {
    constraint_id(schema, &p.module, &p.name, p.check).is_some()
}

/// Returns whether operator `kind` of a migration of `module` drops an object of `held`.
fn drops_any(module: &str, kind: &OperatorKind, held: &[Pending]) -> bool {
    let (q, check) = match kind {
        OperatorKind::DropConstraint(q) => (q, false),
        OperatorKind::DropCheck(q) => (q, true),
        _ => return false,
    };
    let m = q.module.as_ref().map_or(module, |m| m.text.as_str());
    held.iter()
        .any(|p| p.check == check && p.module == m && p.name == q.name.text)
}

/// Returns `source` with the objects of `drops` dropped, at its own version and log.
fn without(source: &Source, drops: &[Pending]) -> Result<Source, String> {
    let mut next = source.clone();
    for p in drops {
        let what = if p.check { "check" } else { "constraint" };
        let text = format!("module {}\ndrop {what} {}\n", p.module, p.name);
        let file = parse_migration(&text, Options::default())
            .ok()
            .map_err(|ds| format!("`{text}` parses: {ds:?}"))?;
        next = next
            .migrate(&file, &text)
            .map_err(|r| format!("`{text}` applies: {:?}", super::reasons(r)))?;
    }
    next.version = source.version;
    next.log.clone_from(&source.log);
    Ok(next)
}

/// Returns `source` checked.
fn checked(source: Source) -> Result<Checked, String> {
    let (schema, program) = source.check().map_err(|ds| {
        format!(
            "{:?}",
            ds.into_iter().map(|d| d.diag.message).collect::<Vec<_>>()
        )
    })?;
    Ok(Checked {
        source,
        schema,
        program: Arc::new(program),
    })
}
