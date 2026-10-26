use adtb_lang::schema::{Generator, Schema, TypeDefKind};
use adtb_lang::types::{Base, RelId, Ty};

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

/// Returns the constructors of the sum `ty`, each with its catalog id and its fields, the header
/// fields first. An anonymous sum's constructors have their positions for ids.
pub fn ctors(schema: &Schema, ty: &Ty) -> Vec<(u32, Vec<(String, Ty)>)> {
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

fn is_record(schema: &Schema, ty: &Ty) -> bool {
    match ty {
        Ty::Data { def, .. } => {
            matches!(schema.ty(*def).kind, TypeDefKind::Sum { record: true, .. })
        }
        _ => false,
    }
}

/// Returns the type of the values of `Key rel`.
pub fn key_value_ty(schema: &Schema, rel: RelId) -> Ty {
    let r = schema.relation(rel);
    let row = &ctors(schema, &r.elem)[0].1;
    let ty = |n: &str| {
        row.iter()
            .find(|(f, _)| f == n)
            .unwrap_or_else(|| panic!("a row of `{}` has key field `{n}`", r.name))
            .1
            .clone()
    };
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

/// Returns the type of an index key component of type `ty`.
pub fn key_type(schema: &Schema, ty: &Ty) -> KeyType {
    match repr(schema, ty) {
        Ty::Base(b) => KeyType::Scalar(scalar(b)),
        Ty::Key { rel, .. } => key_type(schema, &key_value_ty(schema, rel)),
        Ty::Record(fs) => KeyType::AnonRecord(
            AnonRecordType::new(
                fs.iter()
                    .map(|(n, t)| (n.clone(), key_type(schema, t)))
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
                            .map(|(f, t)| (f.clone(), key_type(schema, t)))
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
                .map(|(_, f)| key_type(schema, f))
                .collect(),
        ),
        t => {
            let cs = ctors(schema, &t);
            let keys = OrderKey::spread(cs.len());
            KeyType::Sum(
                SumType::new(
                    cs.iter()
                        .zip(keys)
                        .map(|((_, fs), order_key)| key::CtorType {
                            order_key,
                            fields: fs.iter().map(|(_, f)| key_type(schema, f)).collect(),
                        })
                        .collect(),
                )
                .expect("spread order keys increase"),
            )
        }
    }
}

/// Returns the fields of a row of constructor `ctor` of relation `rel`, the header's first.
pub fn row(schema: &Schema, rel: RelId, ctor: u32) -> Vec<(String, Ty)> {
    ctors(schema, &schema.relation(rel).elem)
        .swap_remove(ctor as usize)
        .1
}
