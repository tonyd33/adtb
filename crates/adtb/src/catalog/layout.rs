use std::collections::BTreeMap;

use adtb_lang::schema::{Schema, TypeDefKind};
use adtb_lang::textual::key_field_ty;
use adtb_lang::types::{Base, RelId, Ty, TypeId};

use crate::heap::{self, FieldType};
use crate::key::{self, AnonRecordType, AnonSumType, CtorId, KeyType, OrderKey, Scalar, SumType};

/// Returns `ty` with every newtype replaced by its representation, down to a base, a sum or
/// another type that is no newtype.
pub fn repr(schema: &Schema, ty: &Ty) -> Ty {
    let mut t = ty.clone();
    while let Ty::Data { def, args, .. } = &t {
        match &schema.ty(*def).kind {
            TypeDefKind::Newtype { base, .. } => t = base.subst(args),
            TypeDefKind::Sum { .. } => break,
        }
    }
    t
}

/// The order key of each constructor of a declared sum, by the constructor's catalog id.
pub type OrderKeys = BTreeMap<u64, OrderKey>;

/// Returns an order key for each constructor of a declared sum of `schema` that `keys` holds
/// none for: spread over the sum when `keys` holds none of its constructors, else between the
/// keys of its neighbours in declaration order.
///
/// Precondition: the keys `keys` holds for each sum's constructors rise in declaration order.
pub fn new_order_keys(schema: &Schema, keys: &OrderKeys) -> OrderKeys {
    let mut out = OrderKeys::new();
    for cs in declared_sums(schema) {
        if !cs.iter().any(|id| keys.contains_key(id)) {
            out.extend(cs.iter().copied().zip(OrderKey::spread(cs.len())));
            continue;
        }
        let mut last: Option<OrderKey> = None;
        for (i, id) in cs.iter().enumerate() {
            if let Some(k) = keys.get(id) {
                last = Some(k.clone());
                continue;
            }
            let next = cs[i + 1..].iter().find_map(|c| keys.get(c));
            let k = OrderKey::between(last.as_ref(), next);
            out.insert(*id, k.clone());
            last = Some(k);
        }
    }
    out
}

/// Returns the catalog ids of the constructors of each declared sum of `schema`, in declaration
/// order.
pub fn declared_sums(schema: &Schema) -> Vec<Vec<u64>> {
    let mut sums: BTreeMap<TypeId, Vec<(u32, u64)>> = BTreeMap::new();
    for (&(def, k), &id) in &schema.ids.ctors {
        if matches!(schema.ty(def).kind, TypeDefKind::Sum { record: false, .. }) {
            sums.entry(def).or_default().push((k, id));
        }
    }
    sums.into_values()
        .map(|mut cs| {
            cs.sort_unstable();
            cs.into_iter().map(|(_, id)| id).collect()
        })
        .collect()
}

/// Returns the constructors of the sum `ty`, each with its catalog id and its fields, the header
/// fields first. An anonymous sum's constructors have their positions for ids. The constructors
/// of `Tag T` are those of `T` without fields.
pub fn ctors(schema: &Schema, ty: &Ty) -> Vec<(u32, Vec<(String, Ty)>)> {
    match repr(schema, ty) {
        Ty::Tag(t) => ctors(schema, &t)
            .into_iter()
            .map(|(id, _)| (id, Vec::new()))
            .collect(),
        Ty::Data { def, args, .. } => {
            let d = schema.ty(def);
            let header: Vec<(String, Ty)> = d
                .header()
                .iter()
                .map(|f| (f.name.clone(), f.ty.subst(&args)))
                .collect();
            d.ctors()
                .iter()
                .enumerate()
                .map(|(k, c)| {
                    let id = schema
                        .ids
                        .ctors
                        .get(&(def, k as u32))
                        .unwrap_or_else(|| panic!("constructor {} has a catalog id", c.name));
                    let mut fs = header.clone();
                    fs.extend(c.fields.iter().map(|f| (f.name.clone(), f.ty.subst(&args))));
                    (ctor_id(*id), fs)
                })
                .collect()
        }
        Ty::Sum { ctors, .. } => ctors
            .into_iter()
            .enumerate()
            .map(|(i, (_, fs))| (i as u32, fs))
            .collect(),
        t => panic!("{t:?} is not a sum"),
    }
}

pub fn ctor_id(id: u64) -> u32 {
    u32::try_from(id).expect("a constructor id fits 32 bits")
}

/// Returns whether `ty`, a type no newtype wraps, is a declared record.
pub fn is_record(schema: &Schema, ty: &Ty) -> bool {
    match ty {
        Ty::Data { def, .. } => {
            matches!(schema.ty(*def).kind, TypeDefKind::Sum { record: true, .. })
        }
        _ => false,
    }
}

/// Returns the type of the values of `Key rel`.
pub fn key_value_ty(schema: &Schema, rel: RelId) -> Ty {
    match schema.relation(rel).key.as_slice() {
        [k] => key_field_ty(schema, rel, k),
        ks => {
            let mut names = ks.to_vec();
            names.sort();
            Ty::Record(
                names
                    .into_iter()
                    .map(|n| {
                        let t = key_field_ty(schema, rel, &n);
                        (n, t)
                    })
                    .collect(),
            )
        }
    }
}

fn scalar(b: Base) -> Scalar {
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

/// Returns how a heap stores a field of type `ty`.
pub fn field_type(schema: &Schema, ty: &Ty) -> FieldType {
    match repr(schema, ty) {
        Ty::Base(b) => FieldType::Scalar(scalar(b)),
        Ty::Key { rel, .. } => field_type(schema, &key_value_ty(schema, rel)),
        Ty::Record(fs) => {
            FieldType::Record(fs.iter().map(|(_, t)| field_type(schema, t)).collect())
        }
        t if is_record(schema, &t) => FieldType::Record(
            ctors(schema, &t)[0]
                .1
                .iter()
                .map(|(_, f)| field_type(schema, f))
                .collect(),
        ),
        t => FieldType::Sum(
            ctors(schema, &t)
                .into_iter()
                .map(|(id, fs)| heap::CtorType {
                    id: CtorId(id),
                    fields: fs.iter().map(|(_, f)| field_type(schema, f)).collect(),
                })
                .collect(),
        ),
    }
}

/// Returns the type of an index key component of type `ty`, each declared sum's constructors
/// encoded by their order keys in `keys`.
pub fn key_type(schema: &Schema, keys: &OrderKeys, ty: &Ty) -> KeyType {
    match repr(schema, ty) {
        Ty::Base(b) => KeyType::Scalar(scalar(b)),
        Ty::Key { rel, .. } => key_type(schema, keys, &key_value_ty(schema, rel)),
        Ty::Record(fs) => KeyType::AnonRecord(
            AnonRecordType::new(
                fs.iter()
                    .map(|(n, t)| (n.clone(), key_type(schema, keys, t)))
                    .collect(),
            )
            .expect("a record's fields have distinct names"),
        ),
        Ty::Sum { ctors, .. } => KeyType::AnonSum(
            AnonSumType::new(
                ctors
                    .iter()
                    .map(|(n, fs)| {
                        let r = fs
                            .iter()
                            .map(|(f, t)| (f.clone(), key_type(schema, keys, t)))
                            .collect();
                        let r = AnonRecordType::new(r).expect("fields have distinct names");
                        (n.clone(), r)
                    })
                    .collect(),
            )
            .expect("a sum's constructors have distinct names"),
        ),
        t if is_record(schema, &t) => KeyType::Record(
            ctors(schema, &t)[0]
                .1
                .iter()
                .map(|(_, f)| key_type(schema, keys, f))
                .collect(),
        ),
        t => KeyType::Sum(
            SumType::new(
                ctors(schema, &t)
                    .into_iter()
                    .map(|(id, fs)| key::CtorType {
                        order_key: keys
                            .get(&u64::from(id))
                            .unwrap_or_else(|| panic!("constructor {id} has an order key"))
                            .clone(),
                        fields: fs.iter().map(|(_, f)| key_type(schema, keys, f)).collect(),
                    })
                    .collect(),
            )
            .expect("order keys follow declaration order"),
        ),
    }
}

/// Returns the fields of a row of constructor `ctor` of relation `rel`, the header's first.
pub fn row(schema: &Schema, rel: RelId, ctor: u32) -> Vec<(String, Ty)> {
    ctors(schema, &schema.relation(rel).elem)
        .swap_remove(ctor as usize)
        .1
}
