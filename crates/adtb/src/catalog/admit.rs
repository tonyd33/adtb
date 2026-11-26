//! The admission of the rows a catalog transaction inserts to the row, value and key limits.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use adtb_lang::ids::CatIndexId;
use adtb_lang::norm::NormalProgram;
use adtb_lang::schema::Schema;
use adtb_lang::types::RelId;

use crate::btree;
use crate::heap::{self, Heap, Oversize, RowLimits};
use crate::key::{KeySchema, Value};
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::change::Data;
use super::roots::RootsRow;
use super::{
    CatRelId, Catalog, Checked, Error, IndexKeys, IndexSpec, OrderKeys, Readers, ctor_id,
    ctor_relations, index_keys, index_program, key_schema, layout, pending, rel_id, row_key,
    row_type, stored,
};

/// Rows of constructor relations, each within the row and value limits of its heap and the key
/// limit of every index over its constructor. Only this module builds one.
#[derive(Debug)]
pub struct Admitted(Data);

impl Admitted {
    pub fn rows(&self) -> &Data {
        &self.0
    }
}

/// Rows of the roots heap, each within its row and value limits. Only this module builds one.
#[derive(Debug)]
pub struct RootsAdmitted(Vec<Vec<Value>>);

impl RootsAdmitted {
    pub fn rows(&self) -> &[Vec<Value>] {
        &self.0
    }
}

/// Admit `rows` to the row and value limits of the roots heap `roots`.
///
/// Fail with [`Error::Refused`] naming each row past one.
pub fn admit_roots(
    roots: &Heap,
    rows: impl IntoIterator<Item = RootsRow>,
) -> Result<RootsAdmitted, Error> {
    let mut why = Vec::new();
    let mut out = Vec::new();
    for row in rows {
        let encoded = row.encode();
        match roots.oversize(&encoded) {
            Ok(()) => out.push(encoded),
            Err(Oversize::Value { len, limit }) => why.push(format!(
                "the roots row holding {} is stored in {len} bytes, over the value limit of {limit}",
                row.what()
            )),
            Err(Oversize::Row { len, limit }) => why.push(format!(
                "the roots row holding {} is stored in {len} bytes, over the row limit of {limit}",
                row.what()
            )),
            Err(Oversize::Unencodable(e)) => return Err(Error::Heap(heap::Error::Unencodable(e))),
        }
    }
    if why.is_empty() {
        Ok(RootsAdmitted(out))
    } else {
        Err(Error::Refused(why))
    }
}

/// The indexes a migration's result keeps, keyed as its build creates them.
pub struct Indexing<'a> {
    held: &'a Schema,
    keys: OrderKeys,
    fresh: OrderKeys,
    normal: Option<NormalProgram>,
    specs: BTreeMap<CatIndexId, IndexSpec>,
    keyed: BTreeMap<CatIndexId, Arc<IndexKeys>>,
}

impl<'a> Indexing<'a> {
    /// Returns the indexes of `next` built over `catalog`, held to the enforced version of
    /// `readers` and kept for its pending drops.
    pub fn new(
        catalog: &Catalog,
        next: &'a Checked,
        readers: &'a Readers,
    ) -> Result<Indexing<'a>, Error> {
        let mut keys = catalog.keys.clone();
        let fresh = layout::new_order_keys(next.schema(), &keys);
        keys.extend(fresh.clone());
        let (compiled, texts) = readers.checking(next.judged());
        let held = compiled.committed();
        let pending = &readers.pending[..];
        let specs = pending::kept_indexes(held, texts, pending).map_err(Error::Internal)?;
        let normal = index_program(compiled, specs.values())?;
        let mut keyed = BTreeMap::new();
        for (id, spec) in &specs {
            keyed.insert(
                *id,
                index_keys(held, normal.as_ref(), spec, &catalog.hosts)?,
            );
        }
        Ok(Indexing {
            held,
            keys,
            fresh,
            normal,
            specs,
            keyed,
        })
    }

    /// Returns the schema the indexes are declared in.
    pub fn held(&self) -> &'a Schema {
        self.held
    }

    /// Returns the order keys of the result: the catalog's, then the fresh ones.
    pub fn keys(&self) -> &OrderKeys {
        &self.keys
    }

    /// Returns the order keys the result adds.
    pub fn fresh(&self) -> &OrderKeys {
        &self.fresh
    }

    pub fn normal(&self) -> Option<&NormalProgram> {
        self.normal.as_ref()
    }

    pub fn specs(&self) -> &BTreeMap<CatIndexId, IndexSpec> {
        &self.specs
    }

    pub fn keyed(&self, id: CatIndexId) -> &Arc<IndexKeys> {
        &self.keyed[&id]
    }

    pub fn key_schema(&self, spec: &IndexSpec) -> Result<KeySchema, Error> {
        key_schema(self.held, self.normal(), &self.keys, spec)
    }
}

/// Admit `data`, the rows a migration gives the relations of `schema` that `rewritten` names, as
/// heaps on `pager` and the indexes of `indexing` store them.
///
/// Fail with [`Error::Refused`] naming each constructor relation whose smallest row passes the row
/// limit, each that `data` gives a row past the row limit or a field past the value limit, and
/// each index that `data` gives a row whose key passes the key limit, each with its first such row.
pub fn admit<R: Runtime>(
    pager: &Pager<R>,
    schema: &Schema,
    rewritten: &BTreeSet<CatRelId>,
    indexing: &Indexing,
    data: Data,
) -> Result<Admitted, Error> {
    let mut why = Vec::new();
    for (r, c) in ctor_relations(schema) {
        let rid = rel_id(schema, r);
        if !rewritten.contains(&rid) {
            continue;
        }
        let rel = schema.relation(r);
        let ctor = &schema.ty(rel.elem_def).ctors()[c as usize].name;
        let name = format!("constructor relation {}.{ctor}", rel.name);
        let limits = match RowLimits::new(pager, row_type(schema, r, c)) {
            Ok(limits) => limits,
            Err(Oversize::Row { len, limit } | Oversize::Value { len, limit }) => {
                why.push(format!(
                    "{name} stores its smallest row in {len} bytes, over the row limit of {limit}"
                ));
                continue;
            }
            Err(Oversize::Unencodable(e)) => return Err(Error::Heap(heap::Error::Unencodable(e))),
        };
        let rows = data
            .get(&rid)
            .and_then(|ctors| ctors.get(&ctor_id(schema, r, c)));
        let past = rows
            .into_iter()
            .flatten()
            .find_map(|row| limits.check(row).err());
        match past {
            Some(Oversize::Row { len, limit }) => why.push(format!(
                "{name} gets a row stored in {len} bytes, over the row limit of {limit}"
            )),
            Some(Oversize::Value { len, limit }) => why.push(format!(
                "{name} gets a row with a field stored in {len} bytes, over the value limit of {limit}"
            )),
            Some(Oversize::Unencodable(e)) => {
                return Err(Error::Heap(heap::Error::Unencodable(e)));
            }
            None => {}
        }
    }
    for (id, spec) in indexing.specs() {
        let rid = rel_id(schema, spec.relation);
        let Some(ctors) = data.get(&rid).filter(|_| rewritten.contains(&rid)) else {
            continue;
        };
        let ks = indexing.key_schema(spec)?;
        'rows: for &c in &spec.ctors {
            let cid = ctor_id(schema, spec.relation, c);
            for row in ctors.get(&cid).into_iter().flatten() {
                let Some(key) = row_key(indexing.keyed(*id), &ks, c, row)? else {
                    continue;
                };
                if let Err(e) = btree::check_key(&ks, pager.page_size(), &key, stored(cid)) {
                    why.push(too_long(schema, spec, &ks, &key, e)?);
                    break 'rows;
                }
            }
        }
    }
    if why.is_empty() {
        Ok(Admitted(data))
    } else {
        Err(Error::Refused(why))
    }
}

impl Catalog {
    /// Admit `rows` of constructor `c` of relation `r` to the row and value limits of its heap and
    /// the key limit of each index over `c`, as heaps and indexes on `pager` store them. Fail at
    /// the first row in order past one.
    pub(super) fn admitted<R: Runtime>(
        &self,
        pager: &Pager<R>,
        r: RelId,
        c: u32,
        rows: impl IntoIterator<Item = Vec<Value>>,
    ) -> Result<Admitted, Error> {
        let rows: Vec<Vec<Value>> = rows.into_iter().collect();
        let cid = ctor_id(self.schema(), r, c);
        for row in &rows {
            self.heap(r, c).check_row(row)?;
            for (spec, index, keys) in self.indexes.values() {
                if spec.relation != r {
                    continue;
                }
                let Some(key) = row_key(keys, index.schema(), c, row)? else {
                    continue;
                };
                if let Err(e) = index.check_key(pager.page_size(), &key, stored(cid)) {
                    let why = too_long(self.schema(), spec, index.schema(), &key, e)?;
                    return Err(Error::Refused(vec![why]));
                }
            }
        }
        let ctors = BTreeMap::from([(cid, rows)]);
        Ok(Admitted(BTreeMap::from([(
            rel_id(self.schema(), r),
            ctors,
        )])))
    }
}

/// Returns the refusal of `e`, met checking key `key` of index `spec` of `schema`, whose key
/// schema is `ks`, against the key limit.
fn too_long(
    schema: &Schema,
    spec: &IndexSpec,
    ks: &KeySchema,
    key: &[Value],
    e: btree::Error,
) -> Result<String, Error> {
    let btree::Error::TooLong { len, max } = e else {
        return Err(e.into());
    };
    let rel = schema.relation(spec.relation);
    let module = &schema.module(rel.module).name;
    let size = ks.encode(key)?.len();
    Ok(format!(
        "index `{module}::{}.{}` needs a key payload of {size} bytes; the limit is {} bytes",
        rel.name,
        spec.clause,
        max - (len - size)
    ))
}
