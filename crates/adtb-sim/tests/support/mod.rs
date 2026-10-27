use std::collections::BTreeMap;
use std::sync::Arc;

use adtb::btree::{BTree, Index};
pub use adtb::catalog::layout::{
    field_type, is_record, key_type, key_value_ty, new_order_keys, repr,
};
use adtb::exec::{self, AtSnapshot, Store, StoredHeap, StoredIndex, Val};
use adtb::heap::{FieldType, Heap, LargeStore};
use adtb::key::{CtorId, Dir, Entry, F32, F64, KeySchema, Tid, Value as KValue};
use adtb::mvcc::{Access, Limits, Scope, Serializer, Transactions};
use adtb::plan::Catalog;
use adtb::storage::Pager;
use adtb_lang::schema::Schema;
use adtb_lang::types::{Base, RelId, Ty};
use adtb_reference::{Database, Value};
use adtb_sim::SimRuntime;
use adtb_sys::lock::Lock;

pub mod harness;
pub mod seeds;

pub use seeds::sweep;

/// A database's pager and transactions, with its heaps and indexes as the executor binds them.
pub type Bound = (
    Pager<SimRuntime>,
    Arc<Transactions<SimRuntime>>,
    BTreeMap<adtb::plan::HeapId, StoredHeap>,
    BTreeMap<adtb::plan::IndexId, StoredIndex>,
);

/// The root pages a database's structures are reopened from.
#[derive(Clone, Debug, Default)]
pub struct Roots {
    pub clog: adtb::storage::Pid,
    pub larges: BTreeMap<RelId, adtb::storage::Pid>,
    pub heaps: BTreeMap<adtb::plan::HeapId, adtb::storage::Pid>,
    pub indexes: BTreeMap<adtb::plan::IndexId, adtb::storage::Pid>,
    /// The write path's page of generated counters, once it has one.
    pub sequences: Option<adtb::storage::Pid>,
}

/// The schema, catalog and rows a database is loaded from.
pub struct World<'a> {
    pub schema: &'a Schema,
    pub catalog: &'a Catalog,
    pub db: &'a Database,
}

/// Returns the constructors of a sum type, each with its header and own fields.
pub fn ctors_of(schema: &Schema, ty: &Ty) -> Vec<(String, Vec<(String, Ty)>)> {
    match repr(schema, ty) {
        Ty::Data { def, args, .. } => {
            let d = schema.ty(def);
            let header: Vec<(String, Ty)> = d
                .header()
                .iter()
                .map(|f| (f.name.clone(), f.ty.subst(&args)))
                .collect();
            d.ctors()
                .iter()
                .map(|c| {
                    let mut fs = header.clone();
                    fs.extend(c.fields.iter().map(|f| (f.name.clone(), f.ty.subst(&args))));
                    (c.name.clone(), fs)
                })
                .collect()
        }
        Ty::Sum { ctors, .. } => ctors,
        Ty::Tag(t) => ctors_of(schema, &t)
            .into_iter()
            .map(|(n, _)| (n, Vec::new()))
            .collect(),
        t => panic!("{t:?} is not a sum"),
    }
}

/// Returns reference value `v` of type `ty` as a heap stores it.
pub fn stored(schema: &Schema, ty: &Ty, v: &Value) -> KValue {
    let t = repr(schema, ty);
    match (&t, v) {
        (Ty::Key { rel, .. }, Value::Key(inner)) => {
            stored(schema, &key_value_ty(schema, *rel), inner)
        }
        (Ty::Base(b), v) => match (b, v) {
            (Base::I8, Value::Int(x)) => KValue::I8(*x as i8),
            (Base::I16, Value::Int(x)) => KValue::I16(*x as i16),
            (Base::I32, Value::Int(x)) => KValue::I32(*x as i32),
            (Base::I64, Value::Int(x)) => KValue::I64(*x),
            (Base::Decimal(_), Value::Decimal(d, _)) => KValue::Decimal(*d),
            (Base::F32, Value::F32(x)) => KValue::F32(F32::new(*x)),
            (Base::F64, Value::F64(x)) => KValue::F64(F64::new(*x)),
            (Base::Text, Value::Text(x)) => KValue::Text(x.clone()),
            (Base::Bytes, Value::Bytes(x)) => KValue::Bytes(x.clone()),
            (Base::SortKey, Value::SortKey(x)) => KValue::SortKey(x.clone()),
            (Base::Instant, Value::Instant(x)) => KValue::Instant(*x),
            (Base::Date, Value::Date(x)) => KValue::Date(*x as i32),
            (Base::Time, Value::Time(x)) => KValue::Time(*x),
            (Base::DateTime, Value::DateTime(x)) => KValue::DateTime(*x),
            (Base::Duration, Value::Duration(x)) => KValue::Duration(*x),
            (Base::ZonedDateTime, Value::Zoned(l, z)) => KValue::ZonedDateTime {
                local: *l,
                zone: z.clone(),
            },
            (Base::Period, Value::Period { months, days }) => KValue::Period {
                months: *months,
                days: *days,
            },
            (b, v) => panic!("{v:?} is not a {b:?}"),
        },
        (Ty::Record(fs), Value::Record(vs)) => KValue::Record(
            fs.iter()
                .zip(vs)
                .map(|((_, t), (_, v))| stored(schema, t, v))
                .collect(),
        ),
        (_, Value::Ctor(c, vs)) => {
            let cs = ctors_of(schema, &t);
            let fields: Vec<KValue> = cs[*c as usize]
                .1
                .iter()
                .map(|(n, f)| {
                    let x = &vs.iter().find(|(m, _)| m == n).unwrap().1;
                    stored(schema, f, x)
                })
                .collect();
            if is_record(schema, &t) {
                KValue::Record(fields)
            } else {
                KValue::Sum { ctor: *c, fields }
            }
        }
        (t, v) => panic!("{v:?} is not a {t:?}"),
    }
}

/// Returns reference value `v` of type `ty` as the executor holds it.
pub fn to_val(schema: &Schema, ty: &Ty, v: &Value) -> Val {
    let t = repr(schema, ty);
    match (&t, v) {
        (Ty::Key { rel, .. }, Value::Key(inner)) => {
            to_val(schema, &key_value_ty(schema, *rel), inner)
        }
        (_, Value::Int(x)) => Val::Int(*x),
        (_, Value::Decimal(d, _)) => Val::Decimal(*d),
        (_, Value::F32(x)) => Val::F32(exec::F32::new(*x)),
        (_, Value::F64(x)) => Val::F64(exec::F64::new(*x)),
        (_, Value::Text(x)) => Val::text(x),
        (_, Value::Bytes(x)) => Val::Bytes(Arc::from(x.as_slice())),
        (_, Value::SortKey(x)) => Val::SortKey(Arc::from(x.as_slice())),
        (_, Value::Instant(x)) => Val::Instant(*x),
        (_, Value::Date(x)) => Val::Date(*x as i32),
        (_, Value::Time(x)) => Val::Time(*x),
        (_, Value::DateTime(x)) => Val::DateTime(*x),
        (_, Value::Duration(x)) => Val::Duration(*x),
        (_, Value::Zoned(l, z)) => Val::Zoned {
            local: *l,
            zone: Arc::from(z.as_str()),
        },
        (_, Value::Period { months, days }) => Val::Period {
            months: *months,
            days: *days,
        },
        (Ty::Record(fs), Value::Record(vs)) => Val::record(
            fs.iter()
                .zip(vs)
                .map(|((_, t), (_, v))| to_val(schema, t, v))
                .collect(),
        ),
        (Ty::Set(e) | Ty::NonEmpty(e), Value::Set(vs)) => {
            Val::set(vs.iter().map(|v| to_val(schema, e, v)).collect())
        }
        (_, Value::Ctor(c, vs)) => {
            let cs = ctors_of(schema, &t);
            Val::ctor(
                *c,
                cs[*c as usize]
                    .1
                    .iter()
                    .map(|(n, f)| to_val(schema, f, &vs.iter().find(|(m, _)| m == n).unwrap().1))
                    .collect(),
            )
        }
        (t, v) => panic!("{v:?} is not a {t:?}"),
    }
}

/// Returns executor value `v` of type `ty` as the reference interpreter holds it.
pub fn to_ref(schema: &Schema, ty: &Ty, v: &Val) -> Value {
    let t = repr(schema, ty);
    match (&t, v) {
        (Ty::Key { rel, .. }, v) => Value::key(to_ref(schema, &key_value_ty(schema, *rel), v)),
        (Ty::Set(e) | Ty::NonEmpty(e), Val::List(xs)) => {
            Value::List(xs.iter().map(|x| to_ref(schema, e, x)).collect())
        }
        (Ty::Set(e) | Ty::NonEmpty(e), Val::Set(xs)) => {
            Value::Set(xs.iter().map(|x| to_ref(schema, e, x)).collect())
        }
        (Ty::Record(fs), Val::Record(xs)) => Value::Record(
            fs.iter()
                .zip(xs.iter())
                .map(|((n, t), x)| (n.clone(), to_ref(schema, t, x)))
                .collect(),
        ),
        (Ty::Base(b), v) => match (b, v) {
            (Base::Decimal(s), Val::Decimal(d)) => Value::Decimal(*d, *s),
            (_, Val::Int(x)) => Value::Int(*x),
            (_, Val::F32(x)) => Value::F32(x.get()),
            (_, Val::F64(x)) => Value::F64(x.get()),
            (_, Val::Text(x)) => Value::Text(x.to_string()),
            (_, Val::Bytes(x)) => Value::Bytes(x.to_vec()),
            (_, Val::SortKey(x)) => Value::SortKey(x.to_vec()),
            (_, Val::Instant(x)) => Value::Instant(*x),
            (_, Val::Date(x)) => Value::Date(i64::from(*x)),
            (_, Val::Time(x)) => Value::Time(*x),
            (_, Val::DateTime(x)) => Value::DateTime(*x),
            (_, Val::Duration(x)) => Value::Duration(*x),
            (_, Val::Zoned { local, zone }) => Value::Zoned(*local, zone.to_string()),
            (_, Val::Period { months, days }) => Value::Period {
                months: *months,
                days: *days,
            },
            (b, v) => panic!("{v:?} is not a {b:?}"),
        },
        (_, Val::Ctor(c, xs)) => {
            let cs = ctors_of(schema, &t);
            Value::Ctor(
                *c,
                cs[*c as usize]
                    .1
                    .iter()
                    .zip(xs.iter())
                    .map(|((n, f), x)| (n.clone(), to_ref(schema, f, x)))
                    .collect(),
            )
        }
        (t, v) => panic!("{v:?} is not a {t:?}"),
    }
}

/// Returns `row` with each field outside the key that is text, an `I64` or an instant changed.
pub fn mutated(schema: &Schema, rel: RelId, row: &Value) -> Option<Value> {
    let Value::Ctor(c, fs) = row else {
        panic!("a row is a constructor value")
    };
    let key = &schema.relation(rel).key;
    let mut changed = false;
    let fs = fs
        .iter()
        .map(|(n, v)| {
            let w = match v {
                _ if key.contains(n) => v.clone(),
                Value::Text(t) => Value::Text(format!("{t}~")),
                Value::Int(i) => Value::Int(i + 1000),
                Value::Instant(i) => Value::Instant(i + 7),
                other => other.clone(),
            };
            changed |= &w != v;
            (n.clone(), w)
        })
        .collect();
    changed.then_some(Value::Ctor(*c, fs))
}

pub struct Db {
    pub pager: Pager<SimRuntime>,
    pub heaps: BTreeMap<adtb::plan::HeapId, Heap>,
    pub rows: BTreeMap<adtb::plan::HeapId, Vec<FieldType>>,
    pub indexes: BTreeMap<adtb::plan::IndexId, (StoredIndex, Vec<String>, RelId)>,
    pub txns: Arc<Transactions<SimRuntime>>,
}

impl Db {
    pub fn create(rt: &SimRuntime, f: &World) -> Db {
        Db::build(rt, f, None).expect("a new database").0
    }

    /// Create the database of `f`, or reopen it at `roots` after recovery. Returns `None` when
    /// the pager cannot open the file.
    pub fn build(rt: &SimRuntime, f: &World, roots: Option<&Roots>) -> Option<(Db, Roots)> {
        let pager = match roots {
            None => Pager::create(rt.clone(), &harness::options(256)).expect("create"),
            Some(_) => Pager::open(rt.clone(), &harness::options(256)).ok()?.0,
        };
        let txns = Arc::new(match roots {
            None => Transactions::create(rt.clone(), &pager).expect("commit log"),
            Some(r) => Transactions::open(rt.clone(), &pager, r.clog).expect("commit log"),
        });
        let mut out = Roots {
            clog: txns.commit_log().root(),
            ..Roots::default()
        };
        let mut heaps = BTreeMap::new();
        let mut rows = BTreeMap::new();
        let mut indexes = BTreeMap::new();
        let keys = new_order_keys(f.schema, &Default::default());
        for (r, d) in &f.catalog.relations {
            let rel = f.schema.relation(*r);
            let large = match roots {
                None => LargeStore::create(&pager, 90_000 + r.0).expect("large store"),
                Some(roots) => LargeStore::open(90_000 + r.0, roots.larges[r]),
            };
            out.larges.insert(*r, large.root());
            let cs = ctors_of(f.schema, &rel.elem);
            for (c, h) in &d.heaps {
                let row: Vec<FieldType> = cs[*c as usize]
                    .1
                    .iter()
                    .map(|(_, t)| field_type(f.schema, t))
                    .collect();
                let heap = match roots {
                    None => Heap::create(&pager, h.0 as u32, row.clone(), large).expect("heap"),
                    Some(roots) => {
                        Heap::open(&pager, h.0 as u32, roots.heaps[h], row.clone(), large)
                            .expect("heap")
                    }
                };
                out.heaps.insert(*h, heap.root());
                heaps.insert(*h, heap);
                rows.insert(*h, row);
            }
            for i in &d.indexes {
                let first = &cs[i.ctors[0] as usize].1;
                let components = i
                    .fields
                    .iter()
                    .map(|n| {
                        let t = &first.iter().find(|(f, _)| f == n).unwrap().1;
                        (key_type(f.schema, &keys, t), Dir::Asc)
                    })
                    .collect();
                let ids: BTreeMap<CtorId, u32> =
                    i.ctors.iter().map(|c| (CtorId(10 + c), *c)).collect();
                let owner = 80_000 + i.id.0 as u32;
                let schema = KeySchema::new(components);
                let ctors = ids.keys().copied().collect();
                let index = match roots {
                    None => Index::create(&pager, owner, schema, ctors).expect("index"),
                    Some(roots) => {
                        Index::open(BTree::open(roots.indexes[&i.id], owner), schema, ctors)
                    }
                };
                out.indexes.insert(i.id, index.tree().root());
                indexes.insert(
                    i.id,
                    (StoredIndex { index, ctors: ids }, i.fields.clone(), *r),
                );
            }
        }
        pager.flush_log();
        Some((
            Db {
                pager,
                heaps,
                rows,
                indexes,
                txns,
            },
            out,
        ))
    }

    /// Returns the heaps and indexes as the executor and the write path bind them.
    pub fn bindings(self) -> Bound {
        let rows = self.rows;
        let heaps = self
            .heaps
            .into_iter()
            .map(|(h, heap)| {
                let row = rows[&h].clone();
                (
                    h,
                    StoredHeap {
                        heap: Arc::new(heap),
                        row,
                    },
                )
            })
            .collect();
        let indexes = self
            .indexes
            .into_iter()
            .map(|(i, (st, _, _))| (i, st))
            .collect();
        (self.pager, self.txns, heaps, indexes)
    }

    pub fn row_ty(f: &World, rel: RelId, c: u32) -> Vec<(String, Ty)> {
        ctors_of(f.schema, &f.schema.relation(rel).elem)[c as usize]
            .1
            .clone()
    }

    pub fn entries(&self, f: &World, rel: RelId, c: u32, row: &Value, tid: Tid) {
        let tys = Db::row_ty(f, rel, c);
        for (st, fields, r) in self.indexes.values() {
            if *r != rel || !st.ctors.values().any(|x| *x == c) {
                continue;
            }
            let key = fields
                .iter()
                .map(|n| {
                    let t = &tys.iter().find(|(m, _)| m == n).unwrap().1;
                    stored(f.schema, t, row.field(n).unwrap())
                })
                .collect();
            st.index
                .insert(
                    &self.pager,
                    &Entry {
                        key,
                        ctor: CtorId(10 + c),
                        tid,
                    },
                )
                .expect("index insert");
        }
    }

    pub fn put(&mut self, f: &World, rel: RelId, c: u32, row: &Value, xid: u64) -> Tid {
        let h = f.catalog.heap(rel, c);
        let tys = Db::row_ty(f, rel, c);
        let Value::Ctor(_, fs) = row else {
            unreachable!()
        };
        let values: Vec<KValue> = tys
            .iter()
            .map(|(n, t)| {
                let v = &fs
                    .iter()
                    .find(|(m, _)| m == n)
                    .unwrap_or_else(|| panic!("{row:?} has field {n} of {tys:?}"))
                    .1;
                stored(f.schema, t, v)
            })
            .collect();
        let tid = self
            .heaps
            .get_mut(&h)
            .unwrap()
            .insert(&self.pager, xid, &values)
            .expect("insert");
        self.entries(f, rel, c, row, tid);
        tid
    }

    pub fn replace(&mut self, f: &World, rel: RelId, c: u32, at: Tid, row: &Value, xid: u64) {
        let h = f.catalog.heap(rel, c);
        let tys = Db::row_ty(f, rel, c);
        let Value::Ctor(_, fs) = row else {
            unreachable!()
        };
        let values: Vec<KValue> = tys
            .iter()
            .map(|(n, t)| stored(f.schema, t, &fs.iter().find(|(m, _)| m == n).unwrap().1))
            .collect();
        let indexed: Vec<usize> = self
            .indexes
            .values()
            .filter(|(st, _, r)| *r == rel && st.ctors.values().any(|x| *x == c))
            .flat_map(|(_, fields, _)| {
                fields
                    .iter()
                    .map(|n| tys.iter().position(|(m, _)| m == n).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect();
        let up = self
            .heaps
            .get_mut(&h)
            .unwrap()
            .update(&self.pager, at, xid, &values, &indexed)
            .expect("update");
        if !up.heap_only {
            self.entries(f, rel, c, row, up.tid);
        }
    }

    pub fn begin(&self) -> u64 {
        self.txns.begin(&self.pager).expect("begin").0
    }

    pub fn commit(&self, x: u64) {
        let lsn = self
            .txns
            .commit(&self.pager, adtb::mvcc::Xid(x))
            .expect("commit");
        self.pager.wait_durable(lsn);
    }

    pub fn abort(&self, x: u64) {
        self.txns
            .abort(&self.pager, adtb::mvcc::Xid(x))
            .expect("abort");
    }

    /// Store the fixture's rows, each under versions no snapshot taken afterwards sees: an
    /// earlier value updated to it, and copies inserted by an aborted, a deleted and a running
    /// transaction.
    pub fn load(rt: &SimRuntime, f: &World) -> Db {
        Db::load_with(rt, f, true)
    }

    /// Store the fixture's rows as [`Db::load`] does, leaving the last transaction running only
    /// when `running`.
    pub fn load_with(rt: &SimRuntime, f: &World, running: bool) -> Db {
        let mut db = Db::create(rt, f);
        let mut all: Vec<(RelId, u32, Value)> = Vec::new();
        for ((r, c), rows) in f.db {
            all.extend(rows.iter().map(|v| (*r, *c, v.clone())));
        }
        let a = db.begin();
        let mut decoys = Vec::new();
        for (r, c, row) in &all {
            match mutated(f.schema, *r, row) {
                Some(m) => {
                    let tid = db.put(f, *r, *c, &m, a);
                    decoys.push((*r, *c, tid, row.clone()));
                }
                None => {
                    db.put(f, *r, *c, row, a);
                }
            }
        }
        db.commit(a);
        let e = db.begin();
        for (r, c, tid, row) in &decoys {
            db.replace(f, *r, *c, *tid, row, e);
        }
        db.commit(e);
        let copy = |r: RelId, row: &Value| mutated(f.schema, r, row).unwrap_or(row.clone());
        let b = db.begin();
        for (r, c, row) in &all {
            db.put(f, *r, *c, &copy(*r, row), b);
        }
        db.abort(b);
        let cx = db.begin();
        let mut gone = Vec::new();
        for (r, c, row) in &all {
            gone.push((*r, *c, db.put(f, *r, *c, &copy(*r, row), cx)));
        }
        db.commit(cx);
        let d = db.begin();
        for (r, c, tid) in gone {
            let h = f.catalog.heap(r, c);
            db.heaps
                .get_mut(&h)
                .unwrap()
                .delete(&db.pager, tid, d)
                .expect("delete");
        }
        db.commit(d);
        if running {
            let x = db.begin();
            for (r, c, row) in &all {
                db.put(f, *r, *c, &copy(*r, row), x);
            }
        }
        db
    }

    pub fn store(self, rt: &SimRuntime) -> Arc<Store<SimRuntime>> {
        let (_, snapshot) = self.txns.take(Scope::Every).expect("a snapshot");
        let mut serializer = Serializer::new(Limits::default());
        let serial = serializer.begin(Access::ReadOnly);
        let reads = AtSnapshot {
            rt: rt.clone(),
            pager: self.pager.clone(),
            txns: self.txns.clone(),
            snapshot,
            reader: None,
            serial: Some((Arc::new(Lock::new(serializer)), serial)),
        };
        let heaps = self
            .heaps
            .into_iter()
            .map(|(h, heap)| {
                let row = self.rows[&h].clone();
                (
                    h,
                    StoredHeap {
                        heap: Arc::new(heap),
                        row,
                    },
                )
            })
            .collect();
        Arc::new(Store {
            pager: self.pager,
            reads: Arc::new(reads),
            heaps,
            indexes: self
                .indexes
                .into_iter()
                .map(|(i, (st, _, _))| (i, st))
                .collect(),
        })
    }
}
