mod plan {
    pub use adtb::plan::*;
}

#[path = "../../adtb/src/plan/tests/fixture.rs"]
#[allow(dead_code)]
mod fixture;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use adtb::batch;
use adtb::btree::Index;
use adtb::exec::{
    self, AtSnapshot, Budget, Call, Error, Exceeded, Options as XOptions, Program, Rows, Scheduler,
    Store, StoredHeap, StoredIndex, Val,
};
use adtb::heap::{CtorType, FieldType, Heap, LargeStore};
use adtb::key::{
    AnonRecordType, AnonSumType, CtorId, Dir, Entry, F32, F64, KeySchema, KeyType, OrderKey,
    Scalar, SumType, Tid, Value as KValue,
};
use adtb::mvcc::{Access, Limits, Scope, Serializer, Transactions};
use adtb::plan::{Options as PlanOptions, Rewrites, plan};
use adtb::storage::{Options, Pager};
use adtb_lang::kernel::Var;
use adtb_lang::schema::{Generator, Schema, TypeDefKind};
use adtb_lang::types::{Base, RelId, Ty};
use adtb_reference::{Abort, R, Value};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_sys::lock::Lock;

use fixture::{Fixture, running};

const PAGE: usize = 4096;

fn options() -> Options {
    Options {
        page_size: PAGE as u32,
        log_block: 512,
        segment_size: 256 * 1024,
        pool_frames: 256,
        ..Options::new("app.adt")
    }
}

fn repr(schema: &Schema, ty: &Ty) -> Ty {
    let mut t = ty.clone();
    while let Ty::Data { def, args, .. } = &t {
        match &schema.ty(*def).kind {
            TypeDefKind::Newtype { base, .. } => t = base.subst(args),
            TypeDefKind::Sum { .. } => break,
        }
    }
    t
}

/// Returns the constructors of a sum type, each with its header and own fields.
fn ctors_of(schema: &Schema, ty: &Ty) -> Vec<(String, Vec<(String, Ty)>)> {
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

fn is_record(schema: &Schema, ty: &Ty) -> bool {
    match repr(schema, ty) {
        Ty::Data { def, .. } => {
            matches!(schema.ty(def).kind, TypeDefKind::Sum { record: true, .. })
        }
        _ => false,
    }
}

/// Returns the type of the values of `Key rel`.
fn key_value_ty(schema: &Schema, rel: RelId) -> Ty {
    let r = schema.relation(rel);
    let row = &ctors_of(schema, &r.elem)[0].1;
    let ty = |n: &str| row.iter().find(|(f, _)| f == n).unwrap().1.clone();
    match r.key.as_slice() {
        [k] => match ty(k) {
            Ty::Key { rel: own, .. } if own == rel => match r.generate {
                Some(Generator::Uuidv7) => Ty::Base(Base::Bytes),
                _ => Ty::Base(Base::I64),
            },
            t => t,
        },
        ks => {
            let mut names = ks.to_vec();
            names.sort();
            Ty::Record(names.iter().map(|n| (n.clone(), ty(n))).collect())
        }
    }
}

fn scalar_of(b: Base) -> Scalar {
    match b {
        Base::I8 => Scalar::I8,
        Base::I16 => Scalar::I16,
        Base::I32 => Scalar::I32,
        Base::I64 => Scalar::I64,
        Base::Decimal(s) => Scalar::Decimal(s),
        Base::F32 => Scalar::F32,
        Base::F64 => Scalar::F64,
        Base::Text => Scalar::Text,
        Base::Bytes => Scalar::Bytes,
        Base::SortKey => Scalar::SortKey,
        Base::Instant => Scalar::Instant,
        Base::Date => Scalar::Date,
        Base::Time => Scalar::Time,
        Base::DateTime => Scalar::DateTime,
        Base::ZonedDateTime => Scalar::ZonedDateTime,
        Base::Duration => Scalar::Duration,
        Base::Period => Scalar::Period,
    }
}

fn field_type(schema: &Schema, ty: &Ty) -> FieldType {
    match repr(schema, ty) {
        Ty::Base(b) => FieldType::Scalar(scalar_of(b)),
        Ty::Key { rel, .. } => field_type(schema, &key_value_ty(schema, rel)),
        Ty::Record(fs) => {
            FieldType::Record(fs.iter().map(|(_, t)| field_type(schema, t)).collect())
        }
        t if is_record(schema, &t) => FieldType::Record(
            ctors_of(schema, &t)[0]
                .1
                .iter()
                .map(|(_, f)| field_type(schema, f))
                .collect(),
        ),
        t => FieldType::Sum(
            ctors_of(schema, &t)
                .iter()
                .enumerate()
                .map(|(i, (_, fs))| CtorType {
                    id: CtorId(i as u32),
                    fields: fs.iter().map(|(_, f)| field_type(schema, f)).collect(),
                })
                .collect(),
        ),
    }
}

fn key_type(schema: &Schema, ty: &Ty) -> KeyType {
    match repr(schema, ty) {
        Ty::Base(b) => KeyType::Scalar(scalar_of(b)),
        Ty::Key { rel, .. } => key_type(schema, &key_value_ty(schema, rel)),
        Ty::Record(fs) => KeyType::AnonRecord(
            AnonRecordType::new(
                fs.iter()
                    .map(|(n, t)| (n.clone(), key_type(schema, t)))
                    .collect(),
            )
            .unwrap(),
        ),
        Ty::Sum { ctors, .. } => KeyType::AnonSum(
            AnonSumType::new(
                ctors
                    .iter()
                    .map(|(n, fs)| {
                        let r = fs
                            .iter()
                            .map(|(f, t)| (f.clone(), key_type(schema, t)))
                            .collect();
                        (n.clone(), AnonRecordType::new(r).unwrap())
                    })
                    .collect(),
            )
            .unwrap(),
        ),
        t if is_record(schema, &t) => KeyType::Record(
            ctors_of(schema, &t)[0]
                .1
                .iter()
                .map(|(_, f)| key_type(schema, f))
                .collect(),
        ),
        t => {
            let cs = ctors_of(schema, &t);
            let keys = OrderKey::spread(cs.len());
            KeyType::Sum(
                SumType::new(
                    cs.iter()
                        .zip(keys)
                        .map(|((_, fs), order_key)| adtb::key::CtorType {
                            order_key,
                            fields: fs.iter().map(|(_, f)| key_type(schema, f)).collect(),
                        })
                        .collect(),
                )
                .unwrap(),
            )
        }
    }
}

/// Returns reference value `v` of type `ty` as a heap stores it.
fn stored(schema: &Schema, ty: &Ty, v: &Value) -> KValue {
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
fn to_val(schema: &Schema, ty: &Ty, v: &Value) -> Val {
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
fn to_ref(schema: &Schema, ty: &Ty, v: &Val) -> Value {
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
fn mutated(schema: &Schema, rel: RelId, row: &Value) -> Option<Value> {
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

struct Db {
    pager: Arc<Pager<SimRuntime>>,
    heaps: BTreeMap<adtb::plan::HeapId, Heap>,
    rows: BTreeMap<adtb::plan::HeapId, Vec<FieldType>>,
    indexes: BTreeMap<adtb::plan::IndexId, (StoredIndex, Vec<String>, RelId)>,
    txns: Arc<Transactions<SimRuntime>>,
}

impl Db {
    fn create(rt: &SimRuntime, f: &Fixture) -> Db {
        let pager = Arc::new(Pager::create(rt.clone(), &options()).expect("create"));
        let txns = Arc::new(Transactions::create(rt.clone(), &pager).expect("commit log"));
        let mut heaps = BTreeMap::new();
        let mut rows = BTreeMap::new();
        let mut indexes = BTreeMap::new();
        for (r, d) in &f.catalog.relations {
            let rel = f.schema.relation(*r);
            let large = LargeStore::create(&pager, 90_000 + r.0).expect("large store");
            let cs = ctors_of(&f.schema, &rel.elem);
            for (c, h) in &d.heaps {
                let row: Vec<FieldType> = cs[*c as usize]
                    .1
                    .iter()
                    .map(|(_, t)| field_type(&f.schema, t))
                    .collect();
                let heap = Heap::create(&pager, h.0 as u32, row.clone(), large).expect("heap");
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
                        (key_type(&f.schema, t), Dir::Asc)
                    })
                    .collect();
                let ids: BTreeMap<CtorId, u32> =
                    i.ctors.iter().map(|c| (CtorId(10 + c), *c)).collect();
                let index = Index::create(
                    &pager,
                    80_000 + i.id.0 as u32,
                    KeySchema::new(components),
                    ids.keys().copied().collect(),
                )
                .expect("index");
                indexes.insert(
                    i.id,
                    (StoredIndex { index, ctors: ids }, i.fields.clone(), *r),
                );
            }
        }
        Db {
            pager,
            heaps,
            rows,
            indexes,
            txns,
        }
    }

    fn row_ty(f: &Fixture, rel: RelId, c: u32) -> Vec<(String, Ty)> {
        ctors_of(&f.schema, &f.schema.relation(rel).elem)[c as usize]
            .1
            .clone()
    }

    fn entries(&self, f: &Fixture, rel: RelId, c: u32, row: &Value, tid: Tid) {
        let tys = Db::row_ty(f, rel, c);
        for (st, fields, r) in self.indexes.values() {
            if *r != rel || !st.ctors.values().any(|x| *x == c) {
                continue;
            }
            let key = fields
                .iter()
                .map(|n| {
                    let t = &tys.iter().find(|(m, _)| m == n).unwrap().1;
                    stored(&f.schema, t, row.field(n).unwrap())
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

    fn put(&mut self, f: &Fixture, rel: RelId, c: u32, row: &Value, xid: u64) -> Tid {
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
                stored(&f.schema, t, v)
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

    fn replace(&mut self, f: &Fixture, rel: RelId, c: u32, at: Tid, row: &Value, xid: u64) {
        let h = f.catalog.heap(rel, c);
        let tys = Db::row_ty(f, rel, c);
        let Value::Ctor(_, fs) = row else {
            unreachable!()
        };
        let values: Vec<KValue> = tys
            .iter()
            .map(|(n, t)| stored(&f.schema, t, &fs.iter().find(|(m, _)| m == n).unwrap().1))
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

    fn begin(&self) -> u64 {
        self.txns.begin(&self.pager).expect("begin").0
    }

    fn commit(&self, x: u64) {
        let lsn = self
            .txns
            .commit(&self.pager, adtb::mvcc::Xid(x))
            .expect("commit");
        self.pager.wait_durable(lsn);
    }

    fn abort(&self, x: u64) {
        self.txns
            .abort(&self.pager, adtb::mvcc::Xid(x))
            .expect("abort");
    }

    /// Store the fixture's rows, each under versions no snapshot taken afterwards sees: an
    /// earlier value updated to it, and copies inserted by an aborted, a deleted and a running
    /// transaction.
    fn load(rt: &SimRuntime, f: &Fixture) -> Db {
        let mut db = Db::create(rt, f);
        let mut all: Vec<(RelId, u32, Value)> = Vec::new();
        for ((r, c), rows) in &f.db {
            all.extend(rows.iter().map(|v| (*r, *c, v.clone())));
        }
        let a = db.begin();
        let mut decoys = Vec::new();
        for (r, c, row) in &all {
            match mutated(&f.schema, *r, row) {
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
        let copy = |r: RelId, row: &Value| mutated(&f.schema, r, row).unwrap_or(row.clone());
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
        let running = db.begin();
        for (r, c, row) in &all {
            db.put(f, *r, *c, &copy(*r, row), running);
        }
        db
    }

    fn store(self, rt: &SimRuntime) -> Arc<Store<SimRuntime>> {
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

fn variants() -> Vec<(&'static str, Rewrites)> {
    let all = Rewrites::ALL;
    vec![
        ("all", all),
        ("none", Rewrites::NONE),
        (
            "index_scan off",
            Rewrites {
                index_scan: false,
                ..all
            },
        ),
        (
            "index_join off",
            Rewrites {
                index_join: false,
                ..all
            },
        ),
        (
            "hash_join off",
            Rewrites {
                hash_join: false,
                ..all
            },
        ),
        (
            "fuse_probe off",
            Rewrites {
                fuse_probe: false,
                ..all
            },
        ),
        (
            "static_route off",
            Rewrites {
                static_route: false,
                ..all
            },
        ),
        (
            "prune off",
            Rewrites {
                prune: false,
                ..all
            },
        ),
        (
            "unswitch off",
            Rewrites {
                unswitch: false,
                ..all
            },
        ),
        (
            "group_agg off",
            Rewrites {
                group_agg: false,
                ..all
            },
        ),
        (
            "semi_naive off",
            Rewrites {
                semi_naive: false,
                ..all
            },
        ),
    ]
}

/// One plan to run, with what the reference interpreter says it computes.
struct Case {
    name: String,
    program: Arc<Program>,
    params: Vec<(Var, Val)>,
    ty: Ty,
    want: R<Value>,
}

/// Returns a case per rewrite variant of ad hoc `text` over `params`, each given as name, type
/// and value.
fn cases_of(f: &Fixture, text: &str, params: &[(&str, &str, Value)]) -> Vec<Case> {
    let types: Vec<(&str, &str)> = params.iter().map(|(n, t, _)| (*n, *t)).collect();
    let (n, t) = f.normal(text, &types);
    let bound: Vec<(Var, Value)> = params
        .iter()
        .map(|(name, _, v)| (n.params[*name], v.clone()))
        .collect();
    let vars: Vec<Var> = bound.iter().map(|(v, _)| *v).collect();
    let want = f.reference(text, params);
    let ty = n.terms.ty(t).clone();
    variants()
        .into_iter()
        .map(|(name, rewrites)| {
            let p = plan(
                &f.schema,
                &f.catalog,
                &n.terms,
                t,
                &vars,
                PlanOptions {
                    rewrites,
                    hiding: false,
                },
            )
            .unwrap_or_else(|e| panic!("`{text}` has no plan: {e}"));
            let program = exec::compile(&f.schema, &f.catalog, &p, None)
                .unwrap_or_else(|e| panic!("`{text}` does not compile: {e:?}"));
            let params = bound
                .iter()
                .map(|(v, x)| (*v, to_val(&f.schema, &p.terms.var_info(*v).ty, x)))
                .collect();
            Case {
                name: format!("`{text}` with {name}"),
                program: Arc::new(program),
                params,
                ty: ty.clone(),
                want: want.clone(),
            }
        })
        .collect()
}

/// Returns the reference outcome an executor outcome stands for.
fn outcome(schema: &Schema, ty: &Ty, got: Result<Val, Error>) -> Result<Value, String> {
    match got {
        Ok(v) => Ok(to_ref(schema, ty, &v)),
        Err(Error::Fault(x)) => Err(format!("fault {}", x.name())),
        Err(Error::StatementLimit(_)) => Err("limit".into()),
        Err(e) => Err(format!("{e:?}")),
    }
}

fn expected(want: &R<Value>) -> Result<Value, String> {
    match want {
        Ok(v) => Ok(v.clone()),
        Err(Abort::Fault(x)) => Err(format!("fault {}", x.name())),
        Err(Abort::Limit(_)) => Err("limit".into()),
        Err(e) => Err(format!("{e:?}")),
    }
}

/// Runs `body` under the simulator over the fixture's rows in heaps and indexes, with
/// `workers` morsel workers.
fn with_store<T>(
    seed: u64,
    f: &Fixture,
    workers: usize,
    body: impl FnOnce(&SimRuntime, &Arc<Store<SimRuntime>>, Option<&Arc<Scheduler<SimRuntime>>>) -> T,
) -> T {
    let mut out = None;
    let mut sim = Sim::new(seed, Config::default());
    let result = sim.run(|rt| {
        let store = Db::load(&rt, f).store(&rt);
        let pool = (workers > 0).then(|| Scheduler::start(rt.clone(), workers));
        out = Some(body(&rt, &store, pool.as_ref()));
        if let Some(p) = pool {
            p.shutdown();
        }
    });
    assert_eq!(Outcome::Finished, result);
    out.expect("the run finished")
}

fn call(
    store: &Arc<Store<SimRuntime>>,
    grant: Arc<exec::Grant>,
    pool: Option<&Arc<Scheduler<SimRuntime>>>,
    options: XOptions,
) -> Call<SimRuntime> {
    Call {
        store: store.clone(),
        grant,
        pool: pool.cloned(),
        options,
        cancel: Arc::new(AtomicBool::new(false)),
    }
}

/// Runs every case under the simulator and returns each case's outcome, and how many tasks the
/// workers ran.
fn run_all(
    seed: u64,
    f: &Fixture,
    cases: &[Case],
    workers: usize,
    opts: XOptions,
) -> (Vec<Result<Value, String>>, usize) {
    with_store(seed, f, workers, |rt, store, pool| {
        let budget = Budget::new(1 << 30);
        let mut got = Vec::new();
        for c in cases {
            let grant = budget.admit(64 << 20).expect("a grant fits");
            let v = exec::execute(
                rt.clone(),
                c.program.clone(),
                call(store, grant, pool, opts),
                &c.params,
            );
            got.push(outcome(&f.schema, &c.ty, v));
        }
        assert_eq!(0, budget.granted(), "every grant returned to the budget");
        (got, pool.map_or(0, |p| p.ran()))
    })
}

/// Checks that every case's outcome under the simulator is the reference interpreter's. Returns
/// how many tasks the workers ran.
fn check(seed: u64, f: &Fixture, cases: &[Case], workers: usize, opts: XOptions) -> usize {
    let (got, ran) = run_all(seed, f, cases, workers, opts);
    if std::env::var("EXEC_SHOW").is_ok() {
        for (c, g) in cases.iter().zip(&got).step_by(11) {
            eprintln!("{} => {g:?}", c.name);
        }
    }
    let mut bad = Vec::new();
    for (c, g) in cases.iter().zip(got) {
        let want = expected(&c.want);
        if want != g {
            bad.push(format!("{}:\n  want {want:?}\n  got  {g:?}", c.name));
        }
    }
    assert!(
        bad.is_empty(),
        "seed {seed}: {} of {} cases differ:\n{}",
        bad.len(),
        cases.len(),
        bad.join("\n")
    );
    ran
}

fn key(i: i64) -> Value {
    Value::key(Value::Int(i))
}

fn uuid(n: u8) -> String {
    format!("identities.key(uuid(\"00000000-0000-7000-8000-0000000000{n:02}\"))")
}

fn ident(f: &Fixture, n: u8) -> Value {
    f.reference(&uuid(n), &[]).expect("a key")
}

const QUERIES: &[&str] = &[
    "from i in identities select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub",
    "from i in identities, u in users where i.user == key u
     select { name: u.name, login: match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub }",
    "from g in identities.Google select g.gid",
    r#"from i in identities where match i | Google g -> g.gid == "root" | _ -> false select key i"#,
    r#"from i in identities.Google where i.gid == "root" select key i"#,
    "from i in identities select i.user.name",
    "from u in users select { name: u.name, n: count(i for i in u.<identities.user) }",
    "from i in identities group i by i.user into g select { user: g.key, n: count(x for x in g) }",
    "from o in orders group o by o.user into g select { user: g.key, total: sum(x.total for x in g) }",
    "from o in orders group o by o.user into g
     select { user: g.key, top: max(x.total for x in g), n: count(x for x in g where x.qty > 0) }",
    "from o in orders group o by o.user into g
     select { user: g.key, ids: from x in g select x.id, n: count(x for x in g) }",
    "from o in orders group o by o.user into g
     select { user: g.key, big: if count(x for x in g) > 1 then sum(x.total for x in g) else 0 }",
    "from u in users select u order by u.name desc",
    "from p in posts select p order by (p.created_at desc, key p) take 2",
    "from u in users
     select { user: u.name,
              latest: top 2 by (p.created_at desc, key p) (from p in posts where p.author == key u select p) }",
    "from o in orders where o.qty > 0 and o.total / o.qty > 10 select o.id",
    "from o in orders where o.total / o.qty > 10 select o.id",
    "from o in orders, u in users where o.total / o.qty > 10 and o.user == key u select u.name",
    "from o in orders, u in users where o.qty > 0 and o.total / o.qty > 10 and o.user == key u select u.name",
    "from u in users select { name: u.name, phones: from d in phones where d.owner == key u select d.seen }",
    "from r in (from u in users select { name: u.name, ds: u.<phones.owner }), d in r.ds
     select { name: r.name, seen: d.seen }",
    "from i in identities, u in users where key u == i.user select u.name",
    "from p in posts select p.author",
    "from u in users, i in u.<identities.user select { name: u.name, id: key i }",
    "from i in identities select match i | Google g -> g | Apple a -> a | EmailPass e -> e",
    "from i in identities.(EmailPass | Google) select match i | EmailPass e -> e.email | Google g -> g.gid",
    "from i in identities group i by tag(i) into g select { kind: g.key, n: count(x for x in g) }",
    "count(x for x in (from i in identities select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub))",
    "from u in users select { name: u.name, top: max(o.total for o in orders where o.user == key u) }",
    "from u in users select { name: u.name, n: count(s for s in u.<sessions.owner) }",
    "from o in orders select match checked_div(o.total, o.qty) | None -> 0 | Some { value: q } -> q",
    "from o in orders select { id: o.id, r: try (o.total / o.qty) }",
    "[1, 2] union (from o in orders select o.qty)",
    "from u in users where exists p in posts where p.author == key u select u.name",
    "from u in users where not exists p in posts where p.author == key u select u.name",
    "from f in follows, g in follows where f.dst == g.src and g.dst == f.src select { a: f.src, b: g.src }",
    "fix a = (from u in users select key u) union (from x in a where { k: x } != { k: me } select x) in a",
    "{ n: count(u for u in users), names: from u in users select u.name }",
    "if exists u in users where u.name == \"bo\" then count(p for p in posts) else 0",
    "from s in sessions select match s.owner | None -> 0 | Some { value: o } -> count(p for p in posts where p.author == o)",
    "from u in users, i in identities where key u == me and i.user == key u select i.id",
    "fix reach = (from f in follows where f.src == me select f.dst)
          union (from r in reach, f in follows where f.src == r select f.dst)
     in from u in users where exists r in reach where r == key u select u",
    "fix even = (from u in users where key u == me select key u)
                union (from o in odd, f in follows where f.src == o select f.dst),
         odd  = from v in even, f in follows where f.src == v select f.dst
     in from u in users where exists o in odd where o == key u select u",
    "fix reach = (from f in follows where f.src == me select f.dst)
               union (from r in reach, f in follows where f.src == r select f.dst)
     in from u in users where not exists r in reach where r == key u select u",
    "sum(o.total for o in orders where o.user == me)",
    "from o in orders select o.total * 2 - o.qty",
    "from u in users select u.name ++ \"!\"",
];

fn running_cases(f: &Fixture) -> Vec<Case> {
    let mut out = Vec::new();
    for me in 1..=3 {
        for q in QUERIES {
            out.extend(cases_of(f, q, &[("me", "Key users", key(me))]));
        }
    }
    let probe = "from i in identities where i.id == k
                 select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub";
    for n in 1..=4 {
        out.extend(cases_of(f, probe, &[("k", "Key identities", ident(f, n))]));
    }
    out
}

#[test]
fn queries_over_heaps_and_indexes_agree_with_the_reference() {
    let f = running();
    let cases = running_cases(&f);
    check(1, &f, &cases, 0, XOptions::default());
}

#[test]
fn morsels_on_several_workers_agree_for_every_seed() {
    let f = running();
    let cases = running_cases(&f);
    let opts = XOptions {
        morsel_rows: 2,
        pack_pages: 0,
        ..XOptions::default()
    };
    let ran: usize = (0..4).map(|seed| check(seed, &f, &cases, 3, opts)).sum();
    assert!(ran > 0, "no worker ran a task");
}

const STOCK: &str = r#"
type Item = { id: Key stock, g: I64, v: I64, name: Text }
relation stock : Item key id generate sequence
  index (g)
type Note = { id: Key notes, item: Key stock, label: Text }
relation notes : Note key id generate sequence
  index (item)
"#;

/// Returns a fixture whose heaps span many pages.
fn stock() -> Fixture {
    let mut f = Fixture::new(STOCK);
    let items: Vec<String> = (1..=300)
        .map(|i| {
            format!(
                r#"Item {{ id: stock.key({i}), g: {}, v: {}, name: "item{i}" }}"#,
                i % 7,
                (i * 37) % 101
            )
        })
        .collect();
    for chunk in items.chunks(40) {
        f.rows("stock", &format!("[{}]", chunk.join(", ")), &[]);
    }
    let notes: Vec<String> = (1..=150)
        .map(|i| {
            format!(
                r#"Note {{ id: notes.key({i}), item: stock.key({}), label: "n{i}" }}"#,
                (i * 3) % 300 + 1
            )
        })
        .collect();
    for chunk in notes.chunks(40) {
        f.rows("notes", &format!("[{}]", chunk.join(", ")), &[]);
    }
    f
}

const STOCK_QUERIES: &[&str] = &[
    "from s in stock where s.v > 50 select s.id",
    "from s in stock group s by s.g into g select { g: g.key, n: count(x for x in g), t: sum(x.v for x in g) }",
    "from s in stock, n in notes where n.item == key s select { s: s.name, n: n.label }",
    "count(s for s in stock)",
    "from s in stock where s.g == 3 select s.v",
    "max(s.v for s in stock)",
    "from s in stock where 100 / (s.v - 50) > 1 select s.id",
    "from s in stock select s order by (s.v desc, key s) take 5",
    "from s in stock select { id: s.id, notes: from n in notes where n.item == key s select n.label }",
];

#[test]
fn morsels_of_heaps_of_many_pages_agree_for_every_seed() {
    let f = stock();
    let mut cases = Vec::new();
    for q in STOCK_QUERIES {
        cases.extend(cases_of(&f, q, &[]));
    }
    let opts = XOptions {
        morsel_rows: 16,
        ..XOptions::default()
    };
    let ran: usize = (0..4).map(|seed| check(seed, &f, &cases, 4, opts)).sum();
    assert!(ran > 0, "no worker ran a task");
}

#[test]
fn a_query_past_its_grant_is_rejected_for_its_statement_limit() {
    let f = stock();
    let cases = cases_of(&f, "from s in stock select s", &[]);
    let c = &cases[0];
    let got = with_store(0, &f, 2, |rt, store, pool| {
        let budget = Budget::new(1 << 30);
        let small = budget.admit(4096).expect("a grant fits");
        let refused = exec::execute(
            rt.clone(),
            c.program.clone(),
            call(store, small, pool, XOptions::default()),
            &[],
        );
        let roomy = budget.admit(64 << 20).expect("a grant fits");
        let done = exec::execute(
            rt.clone(),
            c.program.clone(),
            call(store, roomy, pool, XOptions::default()),
            &[],
        );
        assert_eq!(0, budget.granted(), "every grant returned to the budget");
        (refused, done.map(|v| v.as_set().len()))
    });
    assert_eq!(
        Err(Error::StatementLimit(Exceeded::Memory { limit: 4096 })),
        got.0
    );
    assert_eq!(Ok(300), got.1);
}

#[test]
fn a_fixpoint_past_its_rounds_is_rejected_for_its_statement_limit() {
    let f = running();
    let text = "fix reach = (from f in follows where f.src == me select f.dst)
                union (from r in reach, f in follows where f.src == r select f.dst)
                in reach";
    let cases = cases_of(&f, text, &[("me", "Key users", key(1))]);
    let opts = XOptions {
        fix_rounds: 1,
        ..XOptions::default()
    };
    let got = with_store(0, &f, 0, |rt, store, pool| {
        let budget = Budget::new(1 << 30);
        cases
            .iter()
            .map(|c| {
                let g = budget.admit(1 << 20).unwrap();
                exec::execute(
                    rt.clone(),
                    c.program.clone(),
                    call(store, g, pool, opts),
                    &c.params,
                )
            })
            .collect::<Vec<_>>()
    });
    for (c, g) in cases.iter().zip(got) {
        assert_eq!(
            Err(Error::StatementLimit(Exceeded::Rounds { limit: 1 })),
            g,
            "{}",
            c.name
        );
    }
}

#[test]
fn a_cancelled_call_stops_at_a_morsel_boundary() {
    let f = stock();
    let cases = cases_of(&f, "from s in stock select s.name", &[]);
    let got = with_store(0, &f, 2, |rt, store, pool| {
        let budget = Budget::new(1 << 30);
        let mut c = call(
            store,
            budget.admit(1 << 20).unwrap(),
            pool,
            XOptions::default(),
        );
        c.cancel = Arc::new(AtomicBool::new(true));
        exec::execute(rt.clone(), cases[0].program.clone(), c, &[])
    });
    assert_eq!(Err(Error::Cancelled), got);
}

#[test]
fn a_set_of_rows_comes_back_as_batches_of_the_heap_schema() {
    let f = running();
    let cases = cases_of(&f, "from u in users select u", &[]);
    let users = f.rel("users");
    let row: Vec<FieldType> = Db::row_ty(&f, users, 0)
        .iter()
        .map(|(_, t)| field_type(&f.schema, t))
        .collect();
    let schema = batch::Schema::of_heap(&row);
    let v = with_store(0, &f, 0, |rt, store, pool| {
        let budget = Budget::new(1 << 30);
        exec::execute(
            rt.clone(),
            cases[0].program.clone(),
            call(
                store,
                budget.admit(1 << 20).unwrap(),
                pool,
                XOptions::default(),
            ),
            &[],
        )
    })
    .expect("a value");
    let bs = exec::batches(&v, &schema, Rows::Fields, 2);
    assert_eq!(vec![2, 1], bs.iter().map(|b| b.len()).collect::<Vec<_>>());
    let got: Vec<Vec<KValue>> = bs
        .iter()
        .flat_map(|b| (0..b.len()).map(|i| b.row(i)))
        .collect();
    let tys = Db::row_ty(&f, users, 0);
    let want: Vec<Vec<KValue>> = f.db[&(users, 0)]
        .iter()
        .map(|r| {
            tys.iter()
                .map(|(n, t)| stored(&f.schema, t, r.field(n).unwrap()))
                .collect()
        })
        .collect();
    assert_eq!(want, got);
}

#[test]
fn a_plan_with_a_primitive_the_executor_lacks_is_refused_at_compile_time() {
    let f = running();
    let (n, t) = f.normal("from u in users select lower(u.name)", &[]);
    let p = plan(
        &f.schema,
        &f.catalog,
        &n.terms,
        t,
        &[],
        PlanOptions::default(),
    )
    .unwrap();
    assert!(matches!(
        exec::compile(&f.schema, &f.catalog, &p, None),
        Err(Error::Unsupported(_))
    ));
}

/// Primitives of the prelude the executor does not run yet. Each must agree with the
/// reference interpreter once it does.
#[test]
fn every_primitive_agrees_with_the_reference() {
    let f = running();
    let mut cases = Vec::new();
    let mut refused = Vec::new();
    for q in [
        "from u in users select lower(u.name)",
        "from u in users select upper(u.name)",
        "from u in users select u.name ++ \"\u{301}\"",
        "from o in orders select text(o.total)",
        "from o in orders select div(decimal(o.total, 2), decimal(o.qty + 1, 0), 2, HalfEven)",
        "from u in users select grapheme_len(u.name)",
        "from u in users select matches(u.name, \"^a\")",
        "from u in users select lower(\"ÀÉΣ\" ++ u.name)",
        "from u in users select lower(\"İ\" ++ u.name)",
        "from u in users select upper(u.name ++ \"ßŉ\")",
        "from u in users select fold(u.name ++ \"ẞΣς\")",
        "from u in users select fold(upper(u.name)) == fold(u.name)",
        "from u in users select grapheme_len(u.name ++ \"e\u{301}👍🏽\u{1f1e8}\u{1f1e6}\")",
        "from u in users select matches(u.name, \"(?i)^A\")",
        "from u in users select matches(u.name ++ \"\\n\", \"n$\")",
        "from u in users select matches(u.name ++ \"\\nx\", \"(?m)n$\")",
        "from u in users select matches(u.name ++ \"é\", \"^\\\\p{L}+$\")",
        "from u in users select matches(u.name ++ \"\\n\", \"(?s)n.\")",
        "from u in users select matches(u.name, \"(?U)a+\")",
        "from u in users select collate(u.name, \"en@cldr-49\")",
        "from u in users select collate(u.name ++ \"ö\", \"de-u-ks-level2@cldr-49\")",
        "from u in users select collate(u.name ++ \"ö\", \"sv@cldr-49\")",
        "from u in users select collate(upper(u.name) ++ \" 10\", \"en-u-kn-true-ka-shifted-kf-upper@cldr-49\")",
        "from u in users select collate(u.name, \"und-u-ks-level1-kc-true-kv-punct@cldr-49\")",
        "from u in users select utf8(bytes(u.name ++ \"é\"))",
        "from u in users select match Bytes.parse(\"0x65cc81\") | None -> None | Some b -> utf8(bytes(u.name) ++ b.value)",
        "from u in users select match Bytes.parse(\"0xff\") | None -> None | Some b -> utf8(bytes(u.name) ++ b.value)",
        "from u in users select \"e\" ++ \"\u{301}\" ++ u.name",
        "from o in orders select text(o.qty - 3)",
        "from o in orders select text(decimal(o.total, 2) * decimal(o.qty - 3, 1))",
        "from o in orders select text(f64(o.total) / 7.0)",
        "from o in orders select text(f64(o.total) * 1e300)",
        "from o in orders select text(f64(o.qty) / 1e10)",
        "from o in orders select text(f32(o.qty) / 3.0)",
        "from o in orders select text(f64(o.total) / f64(o.qty))",
        "from o in orders select text(f64(o.qty) / f64(o.qty))",
        "from o in orders select text(-f64(o.total) / f64(o.qty))",
        "from p in posts select text(p.created_at)",
        "from o in orders select text(micros(o.total * 36000000 - 1000))",
        "from o in orders select text(micros(-o.total * 1000000))",
        "from o in orders select text(days(o.qty) - months(o.qty))",
        "from u in users select text(uuid(\"0190A4B2-7C3E-7d4f-9a21-5b6c7d8e9f0\" ++ text(scalar_len(u.name))))",
        "from u in users select try uuid(u.name)",
        "from u in users select print(u)",
        "from i in identities select print(i)",
        "from i in identities select print(tag(i))",
        "from f in follows select print(key f)",
        "from o in orders select print({ total: o.total, user: o.user, half: decimal(o.total, 1) * 0.5 })",
        "from o in orders select print(checked_div(o.total, o.qty))",
        "from u in users select print(u.name ++ \"\\\"\\\\\\n\\r\\t\\u{7}é\")",
        "from p in posts select print(p.created_at - 7d)",
        "from o in orders select print(f64(o.qty) / 0.0)",
        "from u in users select User.parse(print(u))",
        "from i in identities select Identity.parse(print(i))",
        "from f in follows select (Key follows).parse(print(key f))",
        "from u in users select (Key users).parse(\"app::users.key(\" ++ text(scalar_len(u.name)) ++ \")\")",
        "from o in orders select I64.parse(\" -00\" ++ text(o.total) ++ \" \")",
        "from o in orders select I8.parse(text(o.total * 2))",
        "from o in orders select I64.parse(text(o.total) ++ \".0\")",
        "from o in orders select (Decimal 2).parse(text(o.total) ++ \".5\")",
        "from o in orders select (Decimal 2).parse(text(o.total) ++ \".555\")",
        "from o in orders select F64.parse(text(f64(o.total) / 7.0))",
        "from o in orders select F32.parse(text(o.total) ++ \"e37\")",
        "from o in orders select F64.parse(text(o.total) ++ \"e400\")",
        "from o in orders select F64.parse(\"-inf\") == Some { value: -f64(o.qty) / 0.0 }",
        "from u in users select Text.parse(print(u.name ++ \"\\\"\\n\"))",
        "from u in users select Text.parse(\"\\\"e\\\\u{301}\" ++ u.name ++ \"\\\"\")",
        "from u in users select Text.parse(u.name)",
        "from p in posts select Instant.parse(text(p.created_at))",
        "from o in orders select DateTime.parse(\"2026-10-25T09:30:0\" ++ text(o.qty) ++ \".5\")",
        "from o in orders select Date.parse(\"2024-02-2\" ++ text(o.qty + 5))",
        "from o in orders select Date.parse(\"+1000\" ++ text(o.qty) ++ \"-01-01\")",
        "from o in orders select Time.parse(\"23:59:5\" ++ text(o.qty) ++ \".25\")",
        "from o in orders select Duration.parse(text(o.qty * 60) ++ \"s\")",
        "from o in orders select Period.parse(\"P\" ++ text(o.qty) ++ \"M-\" ++ text(o.total) ++ \"D\")",
        "from o in orders select ZonedDateTime.parse(\"2026-03-29T01:3\" ++ text(o.qty) ++ \":00[Europe/Lisbon]\")",
        "from u in users select ZonedDateTime.parse(\"2026-03-29T01:30:00[\" ++ u.name ++ \"]\")",
        "from o in orders select (Set I64).parse(\"[\" ++ text(o.qty) ++ \", 1, 3]\")",
        "from o in orders select (Option I64).parse(\"Some {value:\" ++ text(o.qty) ++ \"}\")",
        "from o in orders select Bool.parse(if o.qty > 2 then \"true\" else \"False\")",
        "from o in orders select (Tag Identity).parse(if o.qty > 2 then \"Google\" else \"Nope\")",
        "from o in orders select try div(decimal(o.total, 2), decimal(o.qty, 1), 4, HalfUp)",
        "from o in orders select div(decimal(-o.total, 0), decimal(o.qty + 2, 0), 0, Floor)",
        "from o in orders select div(decimal(o.total, 0), decimal(o.qty + 2, 3), 1, HalfEven)",
        "from o in orders select try div(decimal(o.total * 10000000000000000, 0), decimal(o.qty + 1, 0), 20, Up)",
        "from o in orders select div(decimal(o.total, 6), decimal(o.qty + 1, 0), 2, HalfUp)",
        "from o in orders select i32(decimal(o.total, 0) * 0.15, HalfEven)",
        "from o in orders select i64(decimal(-o.total, 0) * 0.15, Ceiling)",
        "from o in orders select try i8(f64(o.total) * 1.5, Up)",
        "from o in orders select checked_i8(f64(o.total) * 1.25, HalfDown)",
        "from o in orders select checked_i64(f64(o.total) / f64(o.qty), Down)",
        "from o in orders select decimal(f64(o.total) / 3.0, 4, HalfEven)",
        "from o in orders select decimal(f32(o.qty) / 3.0, 6, Floor)",
        "from o in orders select checked_decimal(f64(o.total) / f64(o.qty), 2, Down)",
        "from o in orders select checked_decimal(f64(o.total) * 1e36, 2, Down)",
        "from o in orders select f64(decimal(o.total, 2) * 0.01)",
        "from o in orders select f32(decimal(o.total, 0) * 0.1)",
        "from p in posts select in_zone(p.created_at, \"Europe/Lisbon\")",
        "from p in posts select in_zone(p.created_at, \"US/Eastern\")",
        "from p in posts select try in_zone(p.created_at, \"europe/lisbon\")",
        "from p in posts select to_instant(in_zone(p.created_at, \"America/New_York\"))",
        "from p in posts select in_zone(p.created_at, \"Asia/Kolkata\") + 1h",
        "from p in posts select in_zone(p.created_at, \"Asia/Kolkata\") - 30min",
        "from o in orders select match DateTime.parse(\"2026-03-29T01:\" ++ text(o.total % 60 + 10) ++ \":00\") | None -> \"none\" | Some d -> print(zoned(d.value, \"Europe/Lisbon\", Clamp))",
        "from o in orders select match DateTime.parse(\"2026-03-29T01:\" ++ text(o.total % 60 + 10) ++ \":00\") | None -> \"none\" | Some d -> print(zoned(d.value, \"Europe/Lisbon\", Reject))",
        "from o in orders select match DateTime.parse(\"2026-10-25T01:\" ++ text(o.total % 60 + 10) ++ \":00\") | None -> \"none\" | Some d -> print(to_instant(zoned(d.value, \"Europe/Lisbon\", Clamp)))",
        "from o in orders select match DateTime.parse(\"2026-10-25T01:\" ++ text(o.total % 60 + 10) ++ \":00\") | None -> \"none\" | Some d -> print(zoned(d.value, \"Europe/Lisbon\", Clamp) + 1h)",
        "from o in orders select match Date.parse(\"2024-01-31\") | None -> \"none\" | Some d -> print(add_period(d.value, months(o.qty) + days(o.qty), Clamp))",
        "from o in orders select match Date.parse(\"2024-01-31\") | None -> \"none\" | Some d -> print(add_period(d.value, months(o.qty), Reject))",
        "from o in orders select match DateTime.parse(\"2024-03-31T12:00:00\") | None -> \"none\" | Some d -> print(add_period(d.value, -months(o.qty), Clamp))",
        "from o in orders select match ZonedDateTime.parse(\"2026-02-28T01:30:00[Europe/Lisbon]\") | None -> \"none\" | Some z -> print(add_period(z.value, days(o.qty + 26), Clamp))",
        "from o in orders select match ZonedDateTime.parse(\"2026-02-28T01:30:00[Europe/Lisbon]\") | None -> \"none\" | Some z -> print(add_period(z.value, days(o.qty + 26), Reject))",
        "from o in orders select span(o.qty, o.total)",
        "from o in orders select try span(o.total, o.qty)",
        "from o in orders select start(span(o.qty, o.total)) + end(span(o.qty, o.total))",
        "from o in orders select overlaps(span(o.qty, o.total), span(40, 60))",
        "from o in orders select covers(span(o.qty, o.total), 50)",
        "from o in orders select covers(span(0, 100), span(o.qty, o.total))",
        "from o in orders select print(span(o.qty, o.total))",
        "from o in orders select (Span I64).parse(\"span(\" ++ text(o.qty) ++ \", 3)\")",
    ] {
        match std::panic::catch_unwind(|| cases_of(&f, q, &[])) {
            Ok(cs) => cases.extend(cs),
            Err(_) => refused.push(q),
        }
    }
    if !cases.is_empty() {
        check(0, &f, &cases, 0, XOptions::default());
    }
    assert!(refused.is_empty(), "refused: {refused:#?}");
}
