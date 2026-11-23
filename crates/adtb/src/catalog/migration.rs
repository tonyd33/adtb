use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::SystemTime;

use adtb_lang::ast::{MigrationFile, OperatorKind};
use adtb_lang::check::Program;
use adtb_lang::depends;
use adtb_lang::evolve::{self, Code, Privilege, Recorded, Resolved, RowMap, Source};
use adtb_lang::kernel;
use adtb_lang::norm::{self, Root};
use adtb_lang::parse::{Options, parse_migration};
use adtb_lang::schema::{Committed, Schema, Stage};
use adtb_lang::textual::{self, Form, key_names};
use adtb_lang::translate::operator_text;
use adtb_lang::types::{RelId, Ty, TypeId};

use crate::btree::Index;
use crate::exec::{self, conv, from_stored_row};
use crate::heap::FieldType;
use crate::key::{CtorId, Value};
use crate::mvcc::{Scope, Transactions};
use crate::plan::HeapId;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::Rot;
use super::admit::{self, Indexing};
use super::codec::{Codec, MigrationRow};
use super::grants::{Dormant, Grantees};
use super::history::{Rebuilding, key_indexes};
use super::hosts::classify;
use super::ids::{stored, type_ctor_id};
use super::roots::RootsRow;
use super::steps::Stepped;
use super::{
    Broken, Built, CatCtorId, CatRelId, Catalog, Change, Checked, Data, ElementTexts, Error, Hash,
    Migrating, Readers, Snapshot, VersionHash, change, ctor_count, ctor_id, ctor_index,
    ctor_relations, delete_keyed, element_types, heap_root, index_ctors, index_keys, indexes,
    layout, moved, oplog, owner, pending, planned, refusal, rel_id, relation, row_key, row_type,
    rows, steps,
};

impl Catalog {
    /// Apply `text`, the migration numbered one past the schema's version, as one transaction:
    /// map the rows of every relation it changes, create the heaps and indexes the new schema
    /// stores them in, write the catalog rows of the new version, and keep its text. Returns once
    /// the migration is durable.
    ///
    /// Fail with [`Error::Refused`], changing nothing, if the migration does not apply to the
    /// schema, if a constraint of the result has no index-bounded incremental check, or if a
    /// constraint or check it adds, or one reading a relation it rewrites, has violating rows in
    /// the final state. Fail with [`Error::RefusedBy`], changing nothing and naming the first
    /// operator the data refuses, if the data contradicts an operator: a constructor or relation
    /// it removes holds rows, a unique or key it adds finds a duplicate, a constructor it
    /// disables holds rows, or a fill, routing or index expression faults on a row. Fail with
    /// [`Error::Internal`] when admitting or validating a constraint breaks a guarantee of the
    /// checker. On any other failure the transaction is aborted, and the catalog must be opened
    /// again.
    ///
    /// Fail with [`Error::Refused`], changing nothing, if the migration adds a grant to a role
    /// `catalog::roles` does not hold. Write, in the migration's transaction, the `Log` rows of
    /// `catalog::grants` for the grants the migration adds, changes and removes.
    ///
    /// Precondition: no `write::Database` holds the page of id counters.
    ///
    /// Postcondition: [`Catalog::open`] after a crash finds either the whole migration or
    /// nothing of it, and no page of the structures the migration replaced or created but did not
    /// commit.
    pub fn migrate<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        text: &str,
    ) -> Result<(), Error> {
        self.migrate_with(pager, txns, text, Grantees::Refuse, true)
            .map(|_| ())
    }

    /// Apply `text` as [`Catalog::migrate`] does, except that a grant to a role
    /// `catalog::roles` does not hold is applied with a dormant row. Returns each dormant row
    /// written, those of the grants of the source that had no row first among them.
    ///
    /// Precondition: no `write::Database` holds the page of id counters.
    pub fn replay<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        text: &str,
    ) -> Result<Vec<Dormant>, Error> {
        self.migrate_with(pager, txns, text, Grantees::Dormant, false)
    }

    /// Replay `text` as [`Catalog::replay`] does, a grant of `held`, by grantee, privilege and
    /// target, applied with a dormant row whether or not its role exists.
    pub fn replay_holding<R: Runtime>(
        &mut self,
        (pager, txns): (&Pager<R>, &Transactions<R>),
        text: &str,
        held: &BTreeSet<(String, Privilege, String)>,
    ) -> Result<Vec<Dormant>, Error> {
        self.held.clone_from(held);
        let replayed = self.replay(pager, txns, text);
        self.held.clear();
        replayed
    }

    fn migrate_with<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        text: &str,
        grantees: Grantees,
        new: bool,
    ) -> Result<Vec<Dormant>, Error> {
        let change = Change::begin(pager, txns, Scope::Every)?.mark(self.root)?;
        let before = (self.source.grants.clone(), self.sequences);
        let applied = self.apply(&change, text, grantees, new);
        let (c, dormant) = match applied {
            Ok(applied) => applied,
            Err(e) => {
                (self.source.grants, self.sequences) = before;
                return Err(change.fail(self, e));
            }
        };
        let sequences = c.sequences;
        let release = match pager.releasing(&self.marks_gone_in(&c)) {
            Ok(release) => release,
            Err((mark, _)) => {
                (self.source.grants, self.sequences) = before;
                let what = self.mark_named(mark);
                return Err(change.fail(self, Error::Quarantined(what)));
            }
        };
        let landed = match change.commit_naming(self.root, |r| r.sequences = sequences, release) {
            Ok(landed) => landed,
            Err(e) => {
                (self.source.grants, self.sequences) = before;
                return Err(e);
            }
        };
        let gone = self.heaps_gone_in(&c);
        *self = c;
        self.end_landed(pager, txns, landed, &gone);
        Ok(dormant)
    }

    /// Returns the catalog `text` migrates this one to in the transaction of `change`, and each
    /// dormant row it writes. First give each grant of the source with no row its row, and
    /// allocate the page of id counters when a grant needs an id and the catalog has none: the
    /// change commits both or neither with the migration.
    fn apply<R: Runtime>(
        &mut self,
        change: &Change<'_, R>,
        text: &str,
        grantees: Grantees,
        new: bool,
    ) -> Result<(Catalog, Vec<Dormant>), Error> {
        let m = change.writes();
        let unrecorded = self.source.grants.iter().any(|g| g.row.is_none());
        if unrecorded && self.sequences().is_none() {
            self.sequences = self.counter_page(m.pager)?;
        }
        let mut dormant = self.record_in(m)?;
        let refused = |why: Vec<String>| Err(Error::Refused(why));
        let file = match parse_migration(text, Options::default()).ok() {
            Ok(f) => f,
            Err(ds) => return refused(ds.into_iter().map(|d| d.message).collect()),
        };
        let (next, (schema, program), recorded) = match self.source.record(&file, text, new) {
            Ok(migrated) => migrated,
            Err(f) => {
                let unstepped = match &f {
                    evolve::Failed::Refused(r) => r.unstepped(),
                    evolve::Failed::Broken(_) => None,
                };
                return Err(match unstepped {
                    Some(k) => {
                        let operator = operator_text(text, &file.operators[k]);
                        refusal(f, |why| Error::RefusedBy { operator, why })
                    }
                    None => refusal(f, Error::Refused),
                });
            }
        };
        if next.grants.iter().any(|g| g.row.is_none()) && self.sequences().is_none() {
            self.sequences = self.counter_page(m.pager)?;
        }
        let operators = match recordable(&next, &recorded) {
            Ok(ops) => oplog::Log::new(&schema).rows(ops),
            Err(why) => return refused(why),
        };
        let next = Checked::new(next, schema, program)?;
        let stepped = steps::stepped(recorded.steps(), &file, text)?;
        let readers = self.held(&file, text, &next, &stepped)?;
        let (schema, program, texts) = readers.checking(next.judged());
        let (old, old_program) = self.checking();
        let changed = depends::changed(old, old_program, schema, program);
        let rewritten = change::rewritten(old, schema, &changed);
        self.admits(schema, program, texts, &rewritten)?;
        let prepared = Prepared {
            next,
            operators,
            grantees,
            changed,
        };
        let (c, written) =
            self.mapped(m, (&file, text, &stepped), prepared, &rewritten, readers)?;
        change.reserve(self, c.next_heap)?;
        dormant.extend(written);
        Ok((c, dormant))
    }

    /// Refuse `schema`, whose checked program is `program` and index element texts `texts`, when
    /// a constraint of it has no index-bounded incremental check over the heaps
    /// [`Catalog::heap_ids`] assigns a migration rewriting the relations `rewritten` names, and
    /// its indexes; each reason.
    fn admits(
        &self,
        schema: &Committed,
        program: &Program,
        texts: &ElementTexts,
        rewritten: &BTreeSet<CatRelId>,
    ) -> Result<(), Error> {
        let normal = norm::program(schema, program).map_err(|e| match e.cause {
            norm::Cause::TooLarge => Error::Refused(vec![e.to_string()]),
            norm::Cause::Stuck(_) | norm::Cause::Invalid(_) => Error::from(Broken::NotNormal(e)),
        })?;
        kernel::validate(schema, &program.terms, &program.body_terms())
            .and_then(|_| kernel::validate(schema, normal.terms(), &normal.roots()))
            .and_then(|_| kernel::validate_normal(schema, normal.terms(), &normal.roots()))
            .map_err(|e| Error::from(Broken::IllTyped(e)))?;
        let ids = self.heap_ids(schema, rewritten)?;
        let heaps: BTreeMap<(RelId, u32), HeapId> = ids
            .into_iter()
            .map(|(rc, id)| (rc, HeapId(u64::from(id))))
            .collect();
        let specs = indexes(schema, texts);
        for spec in specs.values() {
            index_keys(schema, Some(&normal), spec, &self.hosts)?;
        }
        let catalog = planned(schema, specs.values(), &heaps);
        let why = crate::write::refusals(schema, &normal, &catalog)
            .map_err(|e| Error::from(Broken::Admission(e)))?;
        if why.is_empty() {
            return Ok(());
        }
        Err(Error::Refused(
            why.into_iter()
                .map(|(c, detail)| format!("constraint {c} is not admitted: {detail}"))
                .collect(),
        ))
    }

    /// Returns the catalog of `next`, the source after migration `file` of `text`, as `m.x`
    /// writes it: the rows of every relation `rewritten` names mapped through `stepped`, the
    /// steps of each of the migration's operators in turn, from the rows the snapshot of `m.hold`
    /// sees, and the `Log` rows of its grants; and returns too each dormant grant row written.
    fn mapped<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        (file, text, stepped): (&MigrationFile, &str, &Stepped),
        prepared: Prepared,
        rewritten: &BTreeSet<CatRelId>,
        readers: Readers,
    ) -> Result<(Catalog, Vec<Dormant>), Error> {
        let Prepared {
            next,
            operators,
            grantees,
            changed,
        } = prepared;
        let mut data = Data::new();
        for &rid in rewritten {
            let Some(r) = relation(&self.schema, rid) else {
                continue;
            };
            let mut ctors: BTreeMap<CatCtorId, Vec<Vec<Value>>> = (0..ctor_count(&self.schema, r))
                .map(|c| (ctor_id(&self.schema, r, c), Vec::new()))
                .collect();
            for (c, row) in self.rows(&Snapshot::of(m), r)? {
                ctors
                    .get_mut(&ctor_id(&self.schema, r, c))
                    .expect("a constructor of the relation")
                    .push(row);
            }
            data.insert(rid, ctors);
        }
        let mut before: &Schema = &self.schema;
        let mut why = Vec::new();
        let mut blamed = None;
        for (op, steps) in stepped.ops() {
            let held = why.len();
            for (step, schema) in steps {
                why.extend(self.emptied(m, before, schema, &step.maps, &data)?);
                let moved = match moved::moved(before, schema, &step.maps, &data, &self.hosts) {
                    Ok(m) => m,
                    Err(Error::Refused(w)) => {
                        why.extend(w);
                        change::Moved::default()
                    }
                    Err(e) => return Err(e),
                };
                match change::step(before, schema, &mut data, moved) {
                    Ok(()) => {}
                    Err(Error::Refused(w)) => {
                        return Err(Error::RefusedBy {
                            operator: blamed.unwrap_or_else(|| operator_text(text, &op)),
                            why: [why, w].concat(),
                        });
                    }
                    Err(e) => return Err(e),
                }
                before = schema;
            }
            if blamed.is_none() && why.len() > held {
                blamed = Some(operator_text(text, &op));
            }
        }
        let indexing = Indexing::new(self, &next, &readers)?;
        let admitted = match admit::admit(m.pager, &next.schema, rewritten, &indexing, data) {
            Ok(admitted) => Some(admitted),
            Err(Error::Refused(w)) => {
                why.extend(w);
                None
            }
            Err(e) => return Err(e),
        };
        let Some(data) = admitted.filter(|_| why.is_empty()) else {
            return Err(match blamed {
                Some(operator) => Error::RefusedBy { operator, why },
                None => Error::Refused(why),
            });
        };
        let mut built = self.build(
            m,
            &Built {
                next: &next,
                indexing: &indexing,
                data: &data,
                rewritten,
                old: self.objects(),
                operators: &operators,
            },
        )?;
        let mut grants = std::mem::take(&mut built.source.grants);
        let named: BTreeSet<String> = file
            .operators
            .iter()
            .filter_map(|op| match &op.kind {
                OperatorKind::Grant(g) => Some(g.to.text.clone()),
                _ => None,
            })
            .collect();
        let revoked: BTreeSet<String> = file
            .operators
            .iter()
            .filter_map(|op| match &op.kind {
                OperatorKind::Revoke(r) => Some(r.from.text.clone()),
                _ => None,
            })
            .collect();
        let orphans = self.refuse_orphans(m, &built, &grants, &revoked);
        let dormant =
            orphans.and_then(|()| self.write_grants(m, &built, &mut grants, &named, grantees));
        built.source.grants = grants;
        let dormant = dormant?;
        pending::rewrite(m, &self.roots, &self.readers.pending, &readers.pending)?;
        built.readers = readers;
        let why = built.violations(m, self.checking().0, rewritten, &changed)?;
        if !why.is_empty() {
            return Err(Error::Refused(why));
        }
        Ok((built, dormant))
    }

    /// Returns why `data` refuses the step from `a` to `b`, which moves rows by `maps`, beyond
    /// what its data map refuses: a constructor relation it disables holds rows, or a field holds
    /// a value of a constructor of a type that it disables, or that it drops and no map carries.
    fn emptied<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        a: &Schema,
        b: &Schema,
        maps: &[RowMap],
        data: &Data,
    ) -> Result<Vec<String>, Error> {
        let mut why = Vec::new();
        for (r, rid) in a.ids().relation_ids() {
            let Some(rb) = relation(b, rid) else {
                continue;
            };
            for c in 0..ctor_count(a, r) {
                let cid = ctor_id(a, r, c);
                let Some(k) = ctor_index(b, rb, cid) else {
                    continue;
                };
                if a.stage(r, c) == Stage::Declared || b.stage(rb, k) != Stage::Declared {
                    continue;
                }
                let held = match data.get(&rid) {
                    Some(ctors) => ctors.get(&cid).is_some_and(|rows| !rows.is_empty()),
                    None => self.holds(m, rid, cid)?,
                };
                if held {
                    let rel = a.relation(r);
                    let ctor = &a.ty(rel.elem_def).ctors()[c as usize].name;
                    why.push(format!(
                        "constructor relation {}.{ctor} holds rows; it cannot be disabled",
                        rel.name
                    ));
                }
            }
        }
        for (ac, id) in a.ids().ctor_ids() {
            let (def, c) = (ac.def(), ac.index());
            let verb = match b.ids().ctor_of(id) {
                Some(bc)
                    if a.value_stage(def, c) != Stage::Declared
                        && b.value_stage(bc.def(), bc.index()) == Stage::Declared =>
                {
                    "disabled"
                }
                None if !carried(maps, def, c) => "removed",
                _ => continue,
            };
            why.extend(self.stored_in(m, a, data, def, c, verb)?);
        }
        Ok(why)
    }

    /// Returns why constructor `c` of type `def` of `schema` cannot be `verb`: each relation a
    /// field of whose rows holds a value of it.
    fn stored_in<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        schema: &Schema,
        data: &Data,
        def: TypeId,
        c: u32,
        verb: &str,
    ) -> Result<Vec<String>, Error> {
        let id = stored(type_ctor_id(schema, def, c));
        let mut why = Vec::new();
        for (r, rid) in schema.ids().relation_ids() {
            let fields: Vec<Vec<(String, Ty)>> = (0..ctor_count(schema, r))
                .map(|k| layout::row(schema, r, k))
                .collect();
            let reached = fields.iter().flatten().any(|(_, t)| schema.reaches(t, def));
            if !reached {
                continue;
            }
            let types: Vec<Vec<FieldType>> = fields
                .iter()
                .map(|fs| {
                    fs.iter()
                        .map(|(_, t)| layout::field_type(schema, t))
                        .collect()
                })
                .collect();
            let rows: Vec<(u32, Vec<Value>)> = match data.get(&rid) {
                Some(ctors) => ctors
                    .iter()
                    .flat_map(|(&cid, rows)| {
                        let k = ctor_index(schema, r, cid).expect("a constructor of the relation");
                        rows.iter().map(move |row| (k, row.clone()))
                    })
                    .collect(),
                None => match relation(&self.schema, rid) {
                    Some(old) => self.rows(&Snapshot::of(m), old)?,
                    None => continue,
                },
            };
            let held = rows.iter().any(|(k, row)| {
                types[*k as usize]
                    .iter()
                    .zip(row)
                    .any(|(t, v)| holds_ctor(t, v, id))
            });
            if held {
                why.push(format!(
                    "a field of {} holds a value of {}.{}; it cannot be {verb}",
                    schema.relation(r).name,
                    schema.ty(def).name,
                    schema.ty(def).ctors()[c as usize].name
                ));
            }
        }
        Ok(why)
    }

    /// Returns whether the heap of constructor `cid` of relation `rid` holds a row the snapshot
    /// of `m.hold` sees.
    fn holds<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        rid: CatRelId,
        cid: CatCtorId,
    ) -> Result<bool, Error> {
        let Some(r) = relation(&self.schema, rid) else {
            return Ok(false);
        };
        let c = ctor_index(&self.schema, r, cid).expect("a constructor of the relation");
        self.holds_any(&Snapshot::of(m), r, c)
    }

    /// Returns the catalog of `b.next` as transaction `m.x` writes it over this one: a heap for
    /// each constructor relation and an index for each index of the relations `b.rewritten`
    /// names, filled with the rows `b.data` gives them, the structures of every other relation
    /// kept, the roots rows of the replaced structures deleted and those of the new ones
    /// written, and every catalog row that `b.old` holds and the new version does not deleted
    /// and every other one written.
    pub(super) fn build<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        b: &Built,
    ) -> Result<Catalog, Error> {
        let pager = m.pager;
        let schema = b.next.schema.clone();
        m.ids_from(self.next_heap);
        let mut next = Catalog {
            source: b.next.source.clone(),
            hash: VersionHash::of(std::sync::Arc::new(b.next.source.clone())),
            schema,
            program: b.next.program.clone(),
            texts: b.next.texts.clone(),
            root: self.root,
            large: self.large,
            roots: self.roots.clone(),
            heaps: BTreeMap::new(),
            indexes: BTreeMap::new(),
            keys: b.indexing.keys().clone(),
            next_heap: self.next_heap,
            sequences: self.sequences,
            query_memory: self.query_memory,
            readers: Readers::default(),
            hosts: self.hosts.clone(),
            kept: self.kept.clone(),
            held: BTreeSet::new(),
            objects: b.next.objects.clone(),
        };
        let held = b.indexing.held();
        duplicates(b)?;
        let old: BTreeMap<(CatRelId, CatCtorId), (RelId, u32)> = self
            .heaps
            .keys()
            .map(|&(r, c)| {
                let id = (rel_id(&self.schema, r), ctor_id(&self.schema, r, c));
                (id, (r, c))
            })
            .collect();
        let mut named = Vec::new();
        let mut gone = Vec::new();
        for (r, c) in ctor_relations(&next.schema) {
            let rid = rel_id(&next.schema, r);
            let cid = ctor_id(&next.schema, r, c);
            if !b.rewritten.contains(&rid) {
                let kept = old
                    .get(&(rid, cid))
                    .unwrap_or_else(|| panic!("relation {rid} keeps constructor {cid}"));
                next.heaps.insert((r, c), self.heaps[kept].clone());
                continue;
            }
            let heap = m.create_heap(row_type(&next.schema, r, c), self.large)?;
            named.push(heap_root(rid, cid, &heap));
            next.heaps.insert((r, c), heap);
        }
        for (&(rid, cid), rc) in &old {
            if b.rewritten.contains(&rid) {
                gone.push(heap_root(rid, cid, &self.heaps[rc]));
            }
        }
        for (&id, spec) in b.indexing.specs() {
            let rid = rel_id(&next.schema, spec.relation);
            let keys = b.indexing.keyed(id).clone();
            if !b.rewritten.contains(&rid) {
                let (_, index, _) = &self.indexes[&id];
                let index = index.clone();
                next.indexes.insert(id, (spec.clone(), index, keys));
                continue;
            }
            let index = Index::create(
                pager,
                owner(id).ok_or_else(|| {
                    Error::Refused(vec![format!(
                        "index id {id} is past the ids an index takes"
                    )])
                })?,
                b.indexing.key_schema(spec)?,
                index_ctors(held, spec),
            )?;
            named.push(RootsRow::IndexRoot {
                index: id,
                at: index.tree().root(),
            });
            next.indexes.insert(id, (spec.clone(), index, keys));
        }
        for (&id, (spec, index, _)) in &self.indexes {
            if b.rewritten.contains(&rel_id(&self.schema, spec.relation)) {
                gone.push(RootsRow::IndexRoot {
                    index: id,
                    at: index.tree().root(),
                });
            }
        }
        let rebuilding = Rebuilding {
            m,
            old: &self.schema,
            next: &next.schema,
            rewritten: b.rewritten,
            keys: key_indexes(&next.schema, &next.indexes),
        };
        let kept = (self.kept).rebuilt(&rebuilding, &next.heaps, &mut named, &mut gone)?;
        next.kept = kept;
        for (c, key) in b.indexing.fresh() {
            named.push(RootsRow::OrderKey {
                ctor: *c,
                key: key.clone(),
            });
        }
        if let Some(m) = next.source.log.last() {
            named.push(RootsRow::Migration {
                version: m.number,
                text: m.text.clone(),
            });
        }
        self.roots.delete(m, &gone)?;
        let named = self.roots.admit(named)?;
        self.roots.insert_all(m, &named)?;
        next.insert_all(m, b.data)?;
        self.write_rows(m, &next, b)?;
        next.next_heap = m.top.get();
        Ok(next)
    }

    /// Returns the heap id each constructor relation of `schema` has once a migration that
    /// rewrites the relations `rewritten` names is built over this catalog: a kept relation's
    /// heap keeps its id, and every heap of a rewritten one takes the next free id in turn, as
    /// the build draws them.
    fn heap_ids(&self, schema: &Schema, rewritten: &BTreeSet<CatRelId>) -> Result<HeapIds, Error> {
        let old: BTreeMap<(CatRelId, CatCtorId), u32> = self
            .heaps
            .iter()
            .map(|(&(r, c), h)| {
                let id = (rel_id(&self.schema, r), ctor_id(&self.schema, r, c));
                (id, h.id().0)
            })
            .collect();
        let mut next = self.next_heap;
        let mut out = BTreeMap::new();
        for (r, c) in ctor_relations(schema) {
            let rid = rel_id(schema, r);
            let cid = ctor_id(schema, r, c);
            let id = if rewritten.contains(&rid) {
                next += 1;
                next - 1
            } else {
                *old.get(&(rid, cid))
                    .ok_or_else(|| Error::from(Broken::KeptCtor(rid, cid)))?
            };
            out.insert((r, c), id);
        }
        Ok(out)
    }

    /// Write the catalog rows of `next`'s version as `m.x` over this catalog's: delete the stored
    /// row with the key of each row `b.old` holds that the version does not, insert each other row
    /// of the version, and insert the row of its migration.
    fn write_rows<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        next: &Catalog,
        b: &Built,
    ) -> Result<(), Error> {
        let pager = m.pager;
        let old: BTreeMap<CatRelId, Held> = b
            .old
            .iter()
            .map(|(r, rows)| (rel_id(&self.schema, *r), (*r, rows.as_slice())))
            .collect();
        for (&r, rows) in b.next.objects() {
            let rid = rel_id(&next.schema, r);
            let (was, before) = old.get(&rid).copied().unwrap_or((r, &[]));
            let had: HashSet<&(u32, Vec<Value>)> = before.iter().collect();
            let fresh = rows.iter().filter(|&row| !had.contains(row));
            for (c, rows) in by_ctor(fresh) {
                next.insert_all(m, &next.admitted(pager, r, c, rows.into_iter().cloned())?)?;
            }
            let held: HashSet<&(u32, Vec<Value>)> = rows.iter().collect();
            let mut gone: BTreeMap<u32, Vec<Vec<Value>>> = BTreeMap::new();
            for row in before {
                if !held.contains(row) {
                    gone.entry(row.0).or_default().push(row.1.clone());
                }
            }
            for (c, rows) in gone {
                let key = |row: &[Value]| stored_key(&self.schema, was, c, row);
                let keys = rows.iter().map(|row| key(row)).collect();
                delete_keyed(m, self.heap(was, c), keys, key)?;
            }
        }
        if next.source.log.is_empty() {
            return Ok(());
        }
        let previous = self.stored_hash(m)?;
        let hash = oplog::Log::new(&next.schema).chained(previous, b.operators);
        next.hash.made(hash);
        for (r, rows) in rows::recorded(
            &next.schema,
            &next.source,
            b.operators,
            hash,
            micros(pager.wall()),
        ) {
            for (c, rows) in by_ctor(&rows) {
                next.insert_all(m, &next.admitted(pager, r, c, rows.into_iter().cloned())?)?;
            }
        }
        Ok(())
    }

    fn stored_hash<R: Runtime>(&self, m: &Migrating<'_, R>) -> Result<Hash, Error> {
        if self.source.version == 0 {
            return Ok([0; 32]);
        }
        let codec = Codec::<MigrationRow>::new(&self.schema);
        for (c, row) in self.rows(&Snapshot::of(m), codec.relation())? {
            let row = codec.decode(c, &row)?;
            if row.version() == self.source.version {
                return Ok(*row.hash());
            }
        }
        Err(Rot::NoMigration(self.source.version).into())
    }
}

/// Returns `rows` by constructor, each constructor's in order.
fn by_ctor<'a>(
    rows: impl IntoIterator<Item = &'a (u32, Vec<Value>)>,
) -> BTreeMap<u32, Vec<&'a Vec<Value>>> {
    let mut out: BTreeMap<u32, Vec<&Vec<Value>>> = BTreeMap::new();
    for (c, row) in rows {
        out.entry(*c).or_default().push(row);
    }
    out
}

/// Refuse the rows `b.data` gives the relations `b.rewritten` names when two of them share a key
/// of a unique of `b.indexing`: each unique and the keys rows share.
fn duplicates(b: &Built) -> Result<(), Error> {
    let schema = b.indexing.held();
    let normal = b.indexing.normal();
    let mut why = Vec::new();
    for (id, spec) in b.indexing.specs() {
        let rid = rel_id(schema, spec.relation);
        let Some(ctors) = b
            .data
            .rows()
            .get(&rid)
            .filter(|_| spec.unique && b.rewritten.contains(&rid))
        else {
            continue;
        };
        let ks = b.indexing.key_schema(spec)?;
        let mut seen = BTreeSet::new();
        let mut shared = Vec::new();
        for &c in &spec.ctors {
            let types = element_types(schema, normal, spec.relation, c, &spec.elements)
                .map_err(|e| classify(&format!("index `{}`", spec.clause), e, Broken::IndexKey))?;
            for row in &ctors[&ctor_id(schema, spec.relation, c)] {
                if let Some(key) = row_key(b.indexing.keyed(*id), &ks, c, row)?
                    && !seen.insert(ks.encode(&key)?)
                {
                    shared.push(shown_key(schema, &types, &key)?);
                }
            }
        }
        if !shared.is_empty() {
            why.push(format!(
                "rows of {} share the key {} of `{}`",
                schema.relation(spec.relation).name,
                shared.join(", "),
                spec.clause
            ));
        }
    }
    if why.is_empty() {
        Ok(())
    } else {
        Err(Error::Refused(why))
    }
}

/// Returns the components of index key `key`, of types `types`, in the textual syntax of values.
fn shown_key(schema: &Schema, types: &[Ty], key: &[Value]) -> Result<String, Error> {
    let broken = |e| classify("an index key", e, Broken::IndexKey);
    if types.len() != key.len() {
        return Err(broken(
            exec::Broken::KeyComponents {
                key: key.len(),
                types: types.len(),
            }
            .into(),
        ));
    }
    let convs: Vec<_> = types.iter().map(|t| conv(schema, t)).collect();
    let parts = from_stored_row(&convs, key)
        .map_err(broken)?
        .iter()
        .zip(types)
        .map(|(v, t)| textual::print(v, &Form::of(schema, t)).map_err(|e| broken(e.into())))
        .collect::<Result<Vec<String>, Error>>()?;
    Ok(match parts.as_slice() {
        [one] => one.clone(),
        _ => format!("({})", parts.join(", ")),
    })
}

/// Returns whether `v`, stored as `t`, holds a value of the constructor stored as `id`.
fn holds_ctor(t: &FieldType, v: &Value, id: CtorId) -> bool {
    match (t, v) {
        (FieldType::Sum(cs), Value::Sum { ctor, fields }) => {
            cs.get(*ctor as usize).is_some_and(|c| {
                c.id == id
                    || c.fields
                        .iter()
                        .zip(fields)
                        .any(|(t, v)| holds_ctor(t, v, id))
            })
        }
        (FieldType::Record(ts), Value::Record(vs)) => {
            ts.iter().zip(vs).any(|(t, v)| holds_ctor(t, v, id))
        }
        _ => false,
    }
}

/// Returns whether a map of `maps` carries the rows or values of constructor `c` of type `def`.
fn carried(maps: &[RowMap], def: TypeId, c: u32) -> bool {
    maps.iter().any(|m| match m {
        RowMap::Route { def: d, from, .. }
        | RowMap::Merge { def: d, from, .. }
        | RowMap::Nest { def: d, from, .. } => *d == def && from.contains(&c),
        RowMap::Unnest { def: d, from } => *d == def && *from == c,
        _ => false,
    })
}

/// Returns the operators `recorded` resolves of the migration that left `source`, refusing
/// scripts and unavailable function code.
fn recordable<'a>(
    source: &Source,
    recorded: &'a Recorded,
) -> Result<&'a [evolve::ResolvedOp], Vec<String>> {
    let mut why = Vec::new();
    let ops = match recorded.resolved() {
        Err(ds) => {
            why.extend(ds.iter().map(|d| d.message.clone()));
            &[]
        }
        Ok(ops) => ops,
    };
    if ops.iter().any(|o| matches!(o.op, Resolved::Script { .. })) {
        why.push("the engine does not run a `script` yet".to_string());
    }
    match source.functions() {
        Err(ds) => why.extend(ds.into_iter().map(|d| d.diag.message)),
        Ok(fs) => why.extend(fs.iter().filter_map(|f| match &f.code {
            Code::Wasm { module, .. } => Some(format!(
                "function `{}` names Wasm module `{module}`: the engine does not run Wasm functions yet",
                f.name
            )),
            _ => None,
        })),
    }
    if why.is_empty() { Ok(ops) } else { Err(why) }
}

/// Returns `t` in microseconds since 1970-01-01T00:00:00Z, saturating.
fn micros(t: SystemTime) -> i64 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_micros()).unwrap_or(i64::MAX),
        Err(e) => i64::try_from(e.duration().as_micros()).map_or(i64::MIN, |n| -n),
    }
}

/// A relation of the catalog before a migration, with the catalog rows it holds.
type Held<'a> = (RelId, &'a [(u32, Vec<Value>)]);

/// Returns the key of `row`, a row of constructor `c` of relation `r` of `schema`.
fn stored_key(schema: &Schema, r: RelId, c: u32, row: &[Value]) -> Vec<Value> {
    let fields = layout::row(schema, r, c);
    key_names(schema, r)
        .iter()
        .map(|k| {
            let at = fields.iter().position(|(n, _)| n == k).unwrap_or_else(|| {
                panic!("every constructor of a relation has its key field `{k}`")
            });
            row[at].clone()
        })
        .collect()
}

/// The heap id of each constructor relation, by relation and constructor.
type HeapIds = BTreeMap<(RelId, u32), u32>;

struct Prepared {
    next: Checked,
    operators: Vec<oplog::OperatorRow>,
    grantees: Grantees,
    /// The roots [`depends::changed`] finds changed from the schema [`Catalog::checking`]
    /// returns to the one the migration leaves.
    changed: BTreeSet<Root>,
}
