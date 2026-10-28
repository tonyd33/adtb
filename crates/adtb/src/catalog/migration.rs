use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::SystemTime;

use adtb_lang::ast::MigrationFile;
use adtb_lang::check::Program;
use adtb_lang::evolve::{Code, Resolved, RowMap, Source};
use adtb_lang::norm::{self, NormalProgram};
use adtb_lang::parse::{Options, parse_migration};
use adtb_lang::schema::{self, Schema, Stage};
use adtb_lang::types::{RelId, Ty, TypeId};

use crate::btree::Index;
use crate::heap::{self, FieldType, Heap};
use crate::key::{CtorId, Value};
use crate::mvcc::{Scope, Transactions};
use crate::plan::HeapId;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::history::{Rebuilding, key_indexes};
use super::{
    Built, CatCtorId, CatRelId, Catalog, Changing, Checked, Data, Error, HEAP_ROOT, Hash,
    INDEX_ROOT, IndexKeys, IndexSpec, MIGRATION, Migrating, ORDER_KEY, OrderKeys, Readers, change,
    ctor_count, ctor_id, ctor_index, ctor_relations, delete_rows, heap_root, index_ctors,
    index_keys, index_program, indexes, int, key_schema, layout, moved, oplog, owner, pending,
    planned, readers, reasons, rel_id, relation, row_key, row_type, rows, steps, update_root,
    visible,
};

impl Catalog {
    /// Apply `text`, the migration numbered one past the schema's version, as one transaction:
    /// map the rows of every relation it changes, create the heaps and indexes the new schema
    /// stores them in, write the catalog rows of the new version, and keep its text. Returns once
    /// the migration is durable.
    ///
    /// Fail with [`Error::Refused`], changing nothing, if the migration does not apply to the
    /// schema, if a constraint of the result has no index-bounded incremental check, or if the
    /// data contradicts an operator: a constructor or relation it removes holds rows, a unique or
    /// key it adds finds a duplicate, a constructor it disables holds rows, a fill, routing or
    /// index expression faults on a row, or a constraint or check it adds, or one reading a
    /// relation it rewrites, has violating rows in the final state. Fail with
    /// [`Error::Internal`] when admitting or validating a constraint breaks a guarantee of the
    /// checker. On any other failure the transaction is aborted, and the catalog must be opened
    /// again.
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
        let refused = |why: Vec<String>| Err(Error::Refused(why));
        let file = match parse_migration(text, Options::default()).ok() {
            Ok(f) => f,
            Err(ds) => return refused(ds.into_iter().map(|d| d.message).collect()),
        };
        let next = match self.source.migrate(&file, text) {
            Ok(s) => s,
            Err(r) => return refused(reasons(r)),
        };
        let (schema, program) = match next.check() {
            Ok(checked) => checked,
            Err(ds) => return refused(ds.into_iter().map(|d| d.diag.message).collect()),
        };
        let operators = match recordable(&next) {
            Ok(ops) => oplog::operator_rows(&schema, &ops),
            Err(why) => return refused(why),
        };
        let next = Checked {
            source: next,
            schema,
            program: Arc::new(program),
        };
        let readers = self.held(&file, text, &next)?;
        let rewritten = change::rewritten(&self.schema, &next.schema);
        let (schema, program) = readers
            .enforced()
            .map_or((&next.schema, &next.program), |e| (&e.schema, &e.program));
        self.admits(schema, program, &rewritten)?;
        let x = txns.begin(pager)?;
        update_root(pager, self.root, |r| r.changing = Changing::Schema(x))?;
        let (id, snapshot) = txns.take(Scope::Every)?;
        let m = Migrating {
            pager,
            txns,
            x,
            snapshot: &snapshot,
        };
        let prepared = Prepared { next, operators };
        let built = self.replay(&m, &file, text, prepared, &rewritten, readers);
        txns.release(id);
        let built = built.and_then(|c| {
            update_root(pager, self.root, |r| r.next_heap = c.next_heap)?;
            Ok(c)
        });
        match built {
            Ok(c) => {
                txns.commit(pager, x)?;
                *self = c;
            }
            Err(e) => {
                txns.abort(pager, x)?;
                self.free_leaked(pager, txns)?;
                let lsn = update_root(pager, self.root, |r| r.changing = Changing::None)?;
                pager.wait_durable(lsn);
                return Err(e);
            }
        }
        self.free_leaked(pager, txns)?;
        let lsn = update_root(pager, self.root, |r| r.changing = Changing::None)?;
        pager.wait_durable(lsn);
        Ok(())
    }

    /// Refuse `schema`, whose checked program is `program`, when a constraint of it has no
    /// index-bounded incremental check over the heaps [`Catalog::heap_ids`] assigns a migration
    /// rewriting the relations `rewritten` names, and its indexes; each reason.
    fn admits(
        &self,
        schema: &Schema,
        program: &Program,
        rewritten: &BTreeSet<CatRelId>,
    ) -> Result<(), Error> {
        let normal = norm::program(schema, program)
            .map_err(|e| Error::Refused(vec![format!("the schema does not normalise: {e:?}")]))?;
        let (ids, _) = self.heap_ids(schema, rewritten)?;
        let heaps: BTreeMap<(RelId, u32), HeapId> = ids
            .into_iter()
            .map(|(rc, id)| (rc, HeapId(u64::from(id))))
            .collect();
        let specs = indexes(schema);
        for spec in specs.values() {
            index_keys(schema, Some(&normal), spec, &self.hosts)?;
        }
        let catalog = planned(schema, specs.values(), &heaps);
        let why = crate::write::refusals(schema, &normal, &catalog).map_err(Error::Internal)?;
        if why.is_empty() {
            return Ok(());
        }
        Err(Error::Refused(
            why.into_iter()
                .map(|(c, detail)| format!("constraint {c} is not admitted: {detail}"))
                .collect(),
        ))
    }

    /// Returns the catalog of `next`, the source after migration `file`, as `m.x` writes it: the
    /// rows of every relation `rewritten` names mapped through each of the migration's operators
    /// in turn from the rows `m.snapshot` sees.
    fn replay<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        file: &MigrationFile,
        text: &str,
        prepared: Prepared,
        rewritten: &BTreeSet<CatRelId>,
        readers: Readers,
    ) -> Result<Catalog, Error> {
        let Prepared { next, operators } = prepared;
        let mut data = Data::new();
        for &rid in rewritten {
            let Some(r) = relation(&self.schema, rid) else {
                continue;
            };
            let mut ctors: BTreeMap<CatCtorId, Vec<Vec<Value>>> = (0..ctor_count(&self.schema, r))
                .map(|c| (ctor_id(&self.schema, r, c), Vec::new()))
                .collect();
            for (c, row) in self.rows(m.pager, m.txns, m.snapshot, r)? {
                ctors
                    .get_mut(&ctor_id(&self.schema, r, c))
                    .expect("a constructor of the relation")
                    .push(row);
            }
            data.insert(rid, ctors);
        }
        let module = &file.module.text;
        let mut at = self.source.clone();
        at.version += 1;
        at.expecting(module, &file.operators);
        let mut before = self.schema.clone();
        let mut why = Vec::new();
        for op in &file.operators {
            let mut after = at.clone();
            let steps = match after.steps(module, op, text) {
                Ok(s) => s,
                Err(ds) => {
                    return Err(Error::Refused(ds.into_iter().map(|d| d.message).collect()));
                }
            };
            for step in steps {
                let Some(schema) = steps::built(&step, op, text)? else {
                    continue;
                };
                why.extend(self.emptied(m, &before, &schema, &step.maps, &data)?);
                let moved = match moved::moved(&before, &schema, &step.maps, &data, &self.hosts) {
                    Ok(m) => m,
                    Err(Error::Refused(w)) => {
                        why.extend(w);
                        change::Moved::default()
                    }
                    Err(e) => return Err(e),
                };
                match change::step(&before, &schema, &mut data, moved) {
                    Ok(()) => {}
                    Err(Error::Refused(w)) => return Err(Error::Refused([why, w].concat())),
                    Err(e) => return Err(e),
                }
                before = schema;
            }
            at = after;
        }
        why.extend(oversized(m.pager.page_size(), &next.schema, rewritten));
        if !why.is_empty() {
            return Err(Error::Refused(why));
        }
        let mut built = self.build(
            m,
            &Built {
                next,
                data: &data,
                rewritten,
                old: rows::objects(&self.schema, &self.source),
                operators: &operators,
            },
        )?;
        pending::rewrite(m, &self.roots, &self.readers.pending, &readers.pending)?;
        built.readers = readers;
        let why = built.violations(m, self.checking().0, rewritten)?;
        if !why.is_empty() {
            return Err(Error::Refused(why));
        }
        Ok(built)
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
        for &r in a.ids.relations.keys() {
            let rid = rel_id(a, r);
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
        let now: BTreeMap<u64, (TypeId, u32)> =
            b.ids.ctors.iter().map(|(&at, &id)| (id, at)).collect();
        for (&(def, c), &id) in &a.ids.ctors {
            let verb = match now.get(&id) {
                Some(&(d, k))
                    if a.value_stage(def, c) != Stage::Declared
                        && b.value_stage(d, k) == Stage::Declared =>
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
        let id = CatCtorId(schema.ids.ctors[&(def, c)]).stored();
        let mut why = Vec::new();
        for (&r, &rid) in &schema.ids.relations {
            let fields: Vec<Vec<(String, Ty)>> = (0..ctor_count(schema, r))
                .map(|k| layout::row(schema, r, k))
                .collect();
            let reached = fields
                .iter()
                .flatten()
                .any(|(_, t)| readers::reaches(schema, t, def, &mut BTreeSet::new()));
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
            let rid = CatRelId(rid);
            let rows: Vec<(u32, Vec<Value>)> = match data.get(&rid) {
                Some(ctors) => ctors
                    .iter()
                    .flat_map(|(&cid, rows)| {
                        let k = ctor_index(schema, r, cid).expect("a constructor of the relation");
                        rows.iter().map(move |row| (k, row.clone()))
                    })
                    .collect(),
                None => match relation(&self.schema, rid) {
                    Some(old) => self.rows(m.pager, m.txns, m.snapshot, old)?,
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

    /// Returns whether the heap of constructor `cid` of relation `rid` holds a row `m.snapshot`
    /// sees.
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
        Ok(!visible(m.pager, m.txns, m.snapshot, self.heap(r, c))?.is_empty())
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
        let mut keys = self.keys.clone();
        let fresh = layout::new_order_keys(&schema, &keys);
        keys.extend(fresh.clone());
        let (heap_ids, next_heap) = self.heap_ids(&schema, b.rewritten)?;
        let mut next = Catalog {
            source: b.next.source.clone(),
            schema,
            program: b.next.program.clone(),
            root: self.root,
            large: self.large,
            roots: self.roots.clone(),
            heaps: BTreeMap::new(),
            indexes: BTreeMap::new(),
            keys,
            next_heap,
            sequences: self.sequences,
            query_memory: self.query_memory,
            readers: Readers::default(),
            hosts: self.hosts.clone(),
            kept: self.kept.clone(),
        };
        let specs = indexes(&next.schema);
        let normal =
            index_program(&next.schema, &next.program, specs.values()).map_err(Error::Internal)?;
        let mut keyed = BTreeMap::new();
        for (id, spec) in &specs {
            keyed.insert(
                *id,
                index_keys(&next.schema, normal.as_ref(), spec, &self.hosts)?,
            );
        }
        duplicates(&next.schema, &next.keys, normal.as_ref(), &specs, &keyed, b)?;
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
            let heap = Heap::create(
                pager,
                heap_ids[&(r, c)],
                row_type(&next.schema, r, c),
                self.large,
            )?;
            named.push((HEAP_ROOT, heap_root(rid, cid, &heap)));
            next.heaps.insert((r, c), Arc::new(heap));
        }
        for (&(rid, cid), rc) in &old {
            if b.rewritten.contains(&rid) {
                gone.push((HEAP_ROOT, heap_root(rid, cid, &self.heaps[rc])));
            }
        }
        for (id, spec) in specs {
            let rid = rel_id(&next.schema, spec.relation);
            let keys = keyed.remove(&id).expect("every index is keyed");
            if !b.rewritten.contains(&rid) {
                let (_, index, _) = &self.indexes[&id];
                let index = index.clone();
                next.indexes.insert(id, (spec, index, keys));
                continue;
            }
            let index = Index::create(
                pager,
                owner(id),
                key_schema(&next.schema, normal.as_ref(), &next.keys, &spec)?,
                index_ctors(&next.schema, &spec),
            )?;
            named.push((INDEX_ROOT, vec![int(id), int(index.tree().root().0)]));
            next.indexes.insert(id, (spec, index, keys));
        }
        for (&id, (spec, index, _)) in &self.indexes {
            if b.rewritten.contains(&rel_id(&self.schema, spec.relation)) {
                gone.push((INDEX_ROOT, vec![int(id), int(index.tree().root().0)]));
            }
        }
        let rebuilding = Rebuilding {
            pager,
            old: &self.schema,
            next: &next.schema,
            rewritten: b.rewritten,
            keys: key_indexes(&next.schema, &next.indexes),
        };
        let kept = self
            .kept
            .rebuilt(&rebuilding, &mut next.next_heap, &mut named, &mut gone)?;
        next.kept = kept;
        for (c, key) in &fresh {
            named.push((
                ORDER_KEY,
                vec![int(*c), Value::Bytes(key.as_bytes().to_vec())],
            ));
        }
        if let Some(m) = next.source.log.last() {
            named.push((MIGRATION, vec![int(m.number), Value::Text(m.text.clone())]));
        }
        let gone: Vec<Vec<Value>> = gone
            .into_iter()
            .map(|(ctor, fields)| vec![Value::Sum { ctor, fields }])
            .collect();
        delete_rows(m, &self.roots, &gone)?;
        for (ctor, fields) in named {
            self.roots
                .insert(pager, m.x.0, &[Value::Sum { ctor, fields }])?;
        }
        for (&rid, ctors) in b.data {
            let r = relation(&next.schema, rid).expect("data holds relations of the result");
            for (&cid, rows) in ctors {
                let c = ctor_index(&next.schema, r, cid).expect("a constructor of the result");
                for row in rows {
                    next.insert(pager, m.x, r, c, row)?;
                }
            }
        }
        self.write_rows(m, &next, b)?;
        Ok(next)
    }

    /// Returns the heap id each constructor relation of `schema` has once a migration that
    /// rewrites the relations `rewritten` names is built over this catalog, and the id after
    /// the last one it gives: a kept relation's heap keeps its id, and every heap of a rewritten
    /// one takes the next free id in turn.
    fn heap_ids(
        &self,
        schema: &Schema,
        rewritten: &BTreeSet<CatRelId>,
    ) -> Result<(HeapIds, u32), Error> {
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
                *old.get(&(rid, cid)).ok_or_else(|| {
                    Error::Internal(format!("relation {rid} keeps constructor {cid}"))
                })?
            };
            out.insert((r, c), id);
        }
        Ok((out, next))
    }

    /// Write the catalog rows of `next`'s version as `m.x` over this catalog's: delete each row
    /// `b.old` holds that the version does not, insert each other row of the version, and insert
    /// the row of its migration.
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
        for (r, rows) in rows::objects(&next.schema, &next.source) {
            let rid = rel_id(&next.schema, r);
            let (was, before) = old.get(&rid).copied().unwrap_or((r, &[]));
            for (c, row) in &rows {
                if !before.contains(&(*c, row.clone())) {
                    next.insert(pager, m.x, r, *c, row)?;
                }
            }
            let mut gone: BTreeMap<u32, Vec<Vec<Value>>> = BTreeMap::new();
            for (c, row) in before {
                if !rows.contains(&(*c, row.clone())) {
                    gone.entry(*c).or_default().push(row.clone());
                }
            }
            for (c, rows) in gone {
                delete_rows(m, self.heap(was, c), &rows)?;
            }
        }
        if next.source.log.is_empty() {
            return Ok(());
        }
        let previous = self.stored_hash(m)?;
        for (r, rows) in rows::recorded(
            &next.schema,
            &next.source,
            b.operators,
            previous,
            micros(pager.wall()),
        ) {
            for (c, row) in rows {
                next.insert(pager, m.x, r, c, &row)?;
            }
        }
        Ok(())
    }

    fn stored_hash<R: Runtime>(&self, m: &Migrating<'_, R>) -> Result<Hash, Error> {
        if self.source.version == 0 {
            return Ok([0; 32]);
        }
        let module = self
            .schema
            .module_named("catalog")
            .expect("the catalog module");
        let schema::ValueDef::Relation(r) = self.schema.module(module).values["migrations"] else {
            panic!("the catalog declares migrations as a relation");
        };
        for (_, row) in self.rows(m.pager, m.txns, m.snapshot, r)? {
            let [Value::I64(version), Value::Bytes(hash), ..] = row.as_slice() else {
                return Err(Error::Corrupt("malformed migration row".to_string()));
            };
            if *version == i64::try_from(self.source.version).expect("a version fits I64") {
                return hash.as_slice().try_into().map_err(|_| {
                    Error::Corrupt("a migration hash must hold 32 bytes".to_string())
                });
            }
        }
        Err(Error::Corrupt(format!(
            "missing migration {}",
            self.source.version
        )))
    }
}

/// Refuse the rows `b.data` gives the relations `b.rewritten` names when two of them share a key
/// of a unique of `specs`, each keyed by `keyed`: each unique and the keys rows share.
fn duplicates(
    schema: &Schema,
    keys: &OrderKeys,
    normal: Option<&NormalProgram>,
    specs: &BTreeMap<u64, IndexSpec>,
    keyed: &BTreeMap<u64, Arc<IndexKeys>>,
    b: &Built,
) -> Result<(), Error> {
    let mut why = Vec::new();
    for (id, spec) in specs {
        let rid = rel_id(schema, spec.relation);
        let Some(ctors) = b
            .data
            .get(&rid)
            .filter(|_| spec.unique && b.rewritten.contains(&rid))
        else {
            continue;
        };
        let ks = key_schema(schema, normal, keys, spec)?;
        let mut seen = BTreeSet::new();
        let mut shared = Vec::new();
        for &c in &spec.ctors {
            for row in &ctors[&ctor_id(schema, spec.relation, c)] {
                if let Some(key) = row_key(&keyed[id], &ks, c, row)?
                    && !seen.insert(ks.encode(&key))
                {
                    shared.push(shown_key(&key));
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

/// Returns the components of index key `key`, printed.
fn shown_key(key: &[Value]) -> String {
    let parts: Vec<String> = key.iter().map(shown_value).collect();
    match parts.as_slice() {
        [one] => one.clone(),
        _ => format!("({})", parts.join(", ")),
    }
}

fn shown_value(v: &Value) -> String {
    match v {
        Value::Text(t) => format!("{t:?}"),
        Value::I8(x) => x.to_string(),
        Value::I16(x) => x.to_string(),
        Value::I32(x) => x.to_string(),
        Value::I64(x) => x.to_string(),
        Value::Record(vs) => shown_key(vs),
        other => format!("{other:?}"),
    }
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

/// Returns why each constructor relation of `schema` that `rewritten` names cannot store its
/// smallest row on a page of `page_size` bytes.
fn oversized(page_size: usize, schema: &Schema, rewritten: &BTreeSet<CatRelId>) -> Vec<String> {
    let mut why = Vec::new();
    for (r, c) in ctor_relations(schema) {
        if !rewritten.contains(&rel_id(schema, r)) {
            continue;
        }
        if let Err(heap::Error::TooLarge { len, limit }) =
            heap::fits(page_size, row_type(schema, r, c))
        {
            let rel = schema.relation(r);
            let ctor = &schema.ty(rel.elem_def).ctors()[c as usize].name;
            why.push(format!(
                "constructor relation {}.{ctor} stores its smallest row in {len} bytes, over the row limit of {limit}",
                rel.name
            ));
        }
    }
    why
}

/// Resolve the last migration, refusing scripts and unavailable function code.
fn recordable(source: &Source) -> Result<Vec<adtb_lang::evolve::ResolvedOp>, Vec<String>> {
    let mut why = Vec::new();
    let ops = match source.resolved(source.version) {
        Err(ds) => {
            why.extend(ds.into_iter().map(|d| d.message));
            Vec::new()
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
                "function `{}` names Wasm module `{module}`, which no migration installs",
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

/// The heap id of each constructor relation, by relation and constructor.
type HeapIds = BTreeMap<(RelId, u32), u32>;

struct Prepared {
    next: Checked,
    operators: Vec<oplog::OperatorRow>,
}
