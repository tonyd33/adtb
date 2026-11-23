//! Codecs built over a schema, and the stored rows they encode.

use std::marker::PhantomData;

use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;

use crate::exec::Val;
use crate::heap::FieldType;
use crate::key::Value;

use super::super::Rot;
use super::super::ids::{CatCtorId, ctor_id, engine_relation};
use super::super::layout;
use super::{
    Codec, Decode, Field, Fields, Kind, Listed, Nested, Payload, SETTING_IDS, SettingRow, Shape,
    Sum, Table, Variant, coded, counted, same,
};

/// Returns field `name` of `fields`. Fails to compile in a constant when `fields` does not list
/// `name`.
pub const fn field(fields: Fields, name: &str) -> Field {
    let mut at = 0;
    while at < fields.len() {
        if same(fields[at], name) {
            return Field { fields, at };
        }
        at += 1;
    }
    panic!("a codec reads a field its constructor lists")
}

impl Listed {
    /// Returns the layout of `fields` over `declared`, a constructor's fields in declared order.
    ///
    /// Precondition: `fields` lists each name `declared` holds once and no other; `broken` is the
    /// panic message when it does not.
    pub fn new(fields: Fields, declared: &[(String, Ty)], broken: impl Fn() -> String) -> Listed {
        let mut want: Vec<&str> = fields.to_vec();
        let mut have: Vec<&str> = declared.iter().map(|(n, _)| n.as_str()).collect();
        want.sort_unstable();
        have.sort_unstable();
        assert_eq!(want, have, "{}", broken());
        let positions: Vec<usize> = fields
            .iter()
            .map(|f| {
                declared
                    .iter()
                    .position(|(n, _)| n == f)
                    .expect("each listed field is declared")
            })
            .collect();
        let types = positions.iter().map(|&at| declared[at].1.clone()).collect();
        Listed {
            fields,
            positions,
            types,
        }
    }

    pub fn fields(&self) -> Fields {
        self.fields
    }

    /// Returns the handle of each field, in the order [`Listed::fields`] lists them.
    pub fn handles(&self) -> impl Iterator<Item = Field> + use<> {
        let fields = self.fields;
        (0..fields.len()).map(move |at| Field { fields, at })
    }

    fn mine(&self, f: Field) -> usize {
        assert!(
            f.fields == self.fields,
            "a field handle is of the constructor it reads or writes"
        );
        f.at
    }

    /// Returns the declared position of field `f`.
    pub fn position(&self, f: Field) -> usize {
        self.positions[self.mine(f)]
    }

    /// Returns the declared type of field `f`.
    pub fn ty(&self, f: Field) -> &Ty {
        &self.types[self.mine(f)]
    }

    /// Returns `listed`, a value for each field in the order [`Listed::fields`] lists them, in
    /// declared order.
    ///
    /// Precondition: `listed` holds one value for each listed field.
    pub fn declared<V>(&self, listed: Vec<V>) -> Vec<V> {
        assert_eq!(
            listed.len(),
            self.positions.len(),
            "a row gives a value for each listed field"
        );
        let mut slots: Vec<Option<V>> = listed.iter().map(|_| None).collect();
        for (v, &to) in listed.into_iter().zip(&self.positions) {
            slots[to] = Some(v);
        }
        slots
            .into_iter()
            .map(|v| v.expect("`Listed::new` checks the listed fields are the declared ones"))
            .collect()
    }
}

/// Returns the value `given` holds for each field of `fields`, in the order `fields` lists them.
///
/// Precondition: `given` gives each field of `fields` once and no field of another list.
pub fn listed<V>(fields: Fields, given: impl IntoIterator<Item = (Field, V)>) -> Vec<V> {
    let mut slots: Vec<Option<V>> = fields.iter().map(|_| None).collect();
    for (f, v) in given {
        assert!(f.fields == fields, "a row is given fields of its own list");
        assert!(
            slots[f.at].replace(v).is_none(),
            "a row is given field `{}` once",
            fields[f.at]
        );
    }
    slots
        .into_iter()
        .enumerate()
        .map(|(at, v)| v.unwrap_or_else(|| panic!("a row is given field `{}`", fields[at])))
        .collect()
}

/// Returns constructor `id` whose field `fields[n]` holds `kinds[n]`. Fails to compile in a
/// constant when the two differ in length.
pub const fn payload(id: CatCtorId, fields: Fields, kinds: &'static [Kind]) -> Payload {
    assert!(
        fields.len() == kinds.len(),
        "a payload gives each field one kind"
    );
    Payload { id, fields, kinds }
}

impl Nested {
    /// Returns the value of constructor `p` holding `given`, each field `p` lists by its handle
    /// once.
    pub fn of(p: Payload, given: impl IntoIterator<Item = (Field, Nested)>) -> Nested {
        Nested::Ctor {
            id: p.id,
            fields: p.fields,
            values: listed(p.fields, given),
        }
    }

    pub fn text(t: &str) -> Nested {
        Nested::Scalar(Value::Text(t.to_owned()))
    }

    /// Returns the value of `id` as an `I64`.
    pub fn int(id: u64) -> Nested {
        Nested::Scalar(Value::I64(
            i64::try_from(id).expect("a catalog id fits 63 bits"),
        ))
    }
}

impl Table {
    /// Returns the table of `ty`, whose constructors are `ctors` in any order, for field `field`
    /// of constructor `row` of relation `name`, as `at` gives them.
    ///
    /// Preconditions: `ty` is a sum declaring exactly the constructors `ctors` lists, each with
    /// exactly the fields its payload lists, holding what it lists.
    fn new(
        schema: &Schema,
        ty: &Ty,
        ctors: &'static [Payload],
        at: &(&str, &str, CatCtorId),
    ) -> Table {
        let (name, field, row) = *at;
        let FieldType::Sum(cs) = layout::field_type(schema, ty) else {
            panic!("field `{field}` of constructor {row} of {name} is a sum");
        };
        let declared = layout::ctors(schema, ty);
        let mut variants = Vec::new();
        let mut have = Vec::new();
        for (c, (_, fields)) in cs.iter().zip(declared) {
            let id = CatCtorId::from_raw(u64::from(c.id.0));
            have.push(id);
            let Some(&p) = ctors.iter().find(|p| p.id == id) else {
                continue;
            };
            let broken = || {
                format!(
                    "the codec of {name} lists the payload of constructor {id} of field `{field}`"
                )
            };
            let listed = Listed::new(p.fields, &fields, broken);
            let kids = p
                .kinds
                .iter()
                .zip(listed.handles())
                .map(|(k, f)| match k {
                    Kind::Scalar(s) => {
                        assert!(
                            layout::field_type(schema, listed.ty(f)) == FieldType::Scalar(*s),
                            "{}",
                            broken()
                        );
                        None
                    }
                    Kind::Sum(sub) => Some(Table::new(schema, listed.ty(f), sub, at)),
                })
                .collect();
            variants.push(Variant {
                payload: p,
                listed,
                kids,
            });
        }
        let mut want: Vec<CatCtorId> = ctors.iter().map(|p| p.id).collect();
        want.sort_unstable();
        have.sort_unstable();
        assert_eq!(
            want, have,
            "the codec of {name} lists the constructors of field `{field}` of constructor {row}"
        );
        Table(variants)
    }

    fn position(&self, id: CatCtorId) -> Option<u32> {
        let at = self.0.iter().position(|v| v.payload.id == id)?;
        Some(u32::try_from(at).expect("a constructor position fits 32 bits"))
    }

    /// Returns the stored value of `v`.
    ///
    /// Precondition: `v` is a value of the table's type, each constructor given the fields its
    /// payload lists.
    fn stored(&self, v: &Nested) -> Value {
        let Nested::Ctor { id, fields, values } = v else {
            panic!("a sum field holds a constructor, not {v:?}");
        };
        let at = self
            .position(*id)
            .unwrap_or_else(|| panic!("the sum lists constructor {id}"));
        let variant = &self.0[at as usize];
        assert!(
            *fields == variant.payload.fields,
            "constructor {id} is given the fields its payload lists"
        );
        let values = values
            .iter()
            .zip(&variant.kids)
            .map(|(v, kid)| match (kid, v) {
                (None, Nested::Scalar(v)) => v.clone(),
                (Some(t), v) => t.stored(v),
                (None, v) => panic!("field of constructor {id} holds a scalar, not {v:?}"),
            })
            .collect();
        Value::Sum {
            ctor: at,
            fields: variant.listed.declared(values),
        }
    }
}

impl<T: Decode> Codec<T> {
    /// Returns the codec of `T` over `schema`.
    ///
    /// Preconditions: `schema` declares the relation `T` reads with exactly the constructors and
    /// fields `T::CTORS` lists, fields in any order, and each sum field a constructor lists with
    /// exactly the constructors it lists.
    pub fn new(schema: &Schema) -> Codec<T> {
        let r = engine_relation(schema, coded::<T>().id());
        let rel = schema.relation(r);
        let name = format!("{}::{}", schema.module(rel.module).name, rel.name);
        let declared = schema.ty(rel.elem_def).ctors();
        assert_eq!(
            T::CTORS.len(),
            declared.len(),
            "the codec of {name} lists each constructor once"
        );
        let ctors = (0..declared.len())
            .map(|c| {
                let c = u32::try_from(c).expect("a constructor position fits 32 bits");
                let id = ctor_id(schema, r, c);
                let declared_fields = layout::row(schema, r, c);
                let ctor = T::CTORS
                    .iter()
                    .find(|k| k.id == id)
                    .unwrap_or_else(|| panic!("the codec of {name} lists constructor {id}"));
                let listed = Listed::new(ctor.fields, &declared_fields, || {
                    format!("the codec of {name} lists the fields of constructor {id}")
                });
                let mut sums: Vec<Option<Table>> = ctor.fields.iter().map(|_| None).collect();
                for s in ctor.sums {
                    assert!(
                        s.field.fields == ctor.fields,
                        "the codec of {name} lists a sum field of constructor {id} among its fields"
                    );
                    let field = s.field.fields[s.field.at];
                    let at = (name.as_str(), field, id);
                    sums[s.field.at] = Some(Table::new(schema, listed.ty(s.field), s.ctors, &at));
                }
                let types = listed
                    .types
                    .iter()
                    .map(|t| layout::field_type(schema, t))
                    .collect();
                Shape {
                    ctor,
                    name: declared[c as usize].name.clone(),
                    listed,
                    sums,
                    types,
                }
            })
            .collect();
        Codec {
            relation: r,
            name,
            ctors,
            row: PhantomData,
        }
    }

    /// Returns the position of constructor `id` and `listed`, its fields in the order its
    /// [`Ctor`] lists them, in their declared order.
    ///
    /// Fail with [`Rot::CatalogRow`] when the relation has no constructor `id` or `listed` holds
    /// another count of fields than it.
    pub fn declared<V>(&self, id: CatCtorId, listed: Vec<V>) -> Result<(u32, Vec<V>), Rot> {
        let Some(at) = self.ctors.iter().position(|s| s.ctor.id == id) else {
            return Err(Rot::CatalogRow {
                row: self.name.clone(),
                detail: format!("names no constructor {id}"),
            });
        };
        let shape = &self.ctors[at];
        if listed.len() != shape.listed.positions.len() {
            return Err(Rot::CatalogRow {
                row: format!("{}.{}", self.name, shape.name),
                detail: counted(listed.len(), shape.listed.positions.len()),
            });
        }
        Ok((
            u32::try_from(at).expect("a constructor position fits 32 bits"),
            shape.listed.declared(listed),
        ))
    }

    /// Returns the position of constructor `id` and its row holding `given`, in declared order.
    ///
    /// Precondition: the codec lists constructor `id`, and `given` gives each field its [`Ctor`]
    /// lists once and no other.
    pub fn encode<V>(
        &self,
        id: CatCtorId,
        given: impl IntoIterator<Item = (Field, V)>,
    ) -> (u32, Vec<V>) {
        let at = self
            .ctors
            .iter()
            .position(|s| s.ctor.id == id)
            .unwrap_or_else(|| panic!("the codec of {} lists constructor {id}", self.name));
        let shape = &self.ctors[at];
        (
            u32::try_from(at).expect("a constructor position fits 32 bits"),
            shape.listed.declared(listed(shape.ctor.fields, given)),
        )
    }

    /// Returns the position of constructor `id` in the type of sum field `s`.
    ///
    /// Precondition: a constructor of the codec lists `s`, and `s` lists `id`.
    pub fn tag(&self, s: Sum, id: CatCtorId) -> u32 {
        self.table(s)
            .position(id)
            .unwrap_or_else(|| panic!("sum field `{}` lists {id}", s.field.fields[s.field.at]))
    }

    /// Returns the stored value of `v` in sum field `s`.
    ///
    /// Precondition: a constructor of the codec lists `s`, and `v` is a value of its type by the
    /// constructors and fields `s` lists at every level.
    pub fn stored(&self, s: Sum, v: &Nested) -> Value {
        self.table(s).stored(v)
    }

    /// Returns the stored type of field `f`.
    ///
    /// Precondition: a constructor of the codec lists `f`.
    pub fn field_type(&self, f: Field) -> &FieldType {
        let shape = self
            .ctors
            .iter()
            .find(|shape| shape.ctor.fields == f.fields)
            .unwrap_or_else(|| {
                panic!(
                    "a constructor of the codec of {} lists field `{}`",
                    self.name, f.fields[f.at]
                )
            });
        &shape.types[f.at]
    }

    pub(super) fn table(&self, s: Sum) -> &Table {
        self.ctors
            .iter()
            .filter(|shape| shape.ctor.fields == s.field.fields)
            .find_map(|shape| shape.sums[s.field.at].as_ref())
            .unwrap_or_else(|| {
                panic!(
                    "a constructor of the codec of {} lists sum field `{}`",
                    self.name, s.field.fields[s.field.at]
                )
            })
    }

    /// Returns `v` with field `f` of its constructor holding `value`.
    ///
    /// Precondition: [`Codec::decode_val`] reads `v`.
    pub fn with(&self, v: &Val, f: Field, value: Val) -> Val {
        let Val::Ctor(c, fields) = v else {
            panic!("Codec::with takes a value decode_val reads, a constructor's");
        };
        let mut fields = fields.to_vec();
        fields[self.ctors[*c as usize].listed.position(f)] = value;
        Val::ctor(*c, fields)
    }
}

impl Codec<SettingRow> {
    /// Returns the row of the setting constructor at position `c`, in declared order: its header
    /// field `id` holding `id` of the constructor's catalog id, and each other field the value
    /// `own` returns for its name.
    ///
    /// Precondition: the relation has a constructor at `c`.
    pub fn setting<V, E>(
        &self,
        c: u32,
        id: impl FnOnce(i64) -> V,
        mut own: impl FnMut(&'static str) -> Result<V, E>,
    ) -> Result<Vec<V>, E> {
        let shape = &self.ctors[c as usize];
        let header = *SETTING_IDS
            .iter()
            .find(|f| f.fields == shape.ctor.fields)
            .expect("`SETTING_IDS` holds the header of each setting's fields");
        let raw = i64::try_from(shape.ctor.id.raw()).expect("a catalog id fits 63 bits");
        let mut given = vec![(header, id(raw))];
        for f in shape.listed.handles().filter(|f| f.at != header.at) {
            given.push((f, own(f.fields[f.at])?));
        }
        Ok(shape.listed.declared(listed(shape.ctor.fields, given)))
    }
}
