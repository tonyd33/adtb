//! Catalog rows as typed values: one codec per catalog relation, reading and writing a row's
//! constructor by its fixed id and each field by its declared name.

use std::marker::PhantomData;

use adtb_lang::evolve::{DeployedGrant, GrantRow, Privilege};
use adtb_lang::ids::{engine_ctor as k, engine_rel};
use adtb_lang::schema::Schema;
use adtb_lang::schema::prelude::ctor;
use adtb_lang::types::{RelId, Ty};

use crate::exec::{self, Val};
use crate::heap::FieldType;
use crate::key::{F64, Scalar, Value};

use super::Rot;
use super::ids::{CatCtorId, CatRelId, ctor_id, engine_relation};
use super::layout;
use super::oplog::Hash;

#[cfg(test)]
#[path = "codec_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "codec_testing.rs"]
pub(crate) mod testing;

/// The names of all the fields of one constructor, the header's included.
pub type Fields = &'static [&'static str];

/// One field of a constructor's [`Fields`].
#[derive(Clone, Copy, Debug)]
pub struct Field {
    fields: Fields,
    at: usize,
}

const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

impl Field {
    /// Returns the fields this is one of.
    pub fn fields(self) -> Fields {
        self.fields
    }

    /// Returns the position of this field among [`Field::fields`].
    pub fn at(self) -> usize {
        self.at
    }
}

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

/// The declared position and type of each field one constructor's [`Fields`] lists.
pub struct Listed {
    fields: Fields,
    positions: Vec<usize>,
    types: Vec<Ty>,
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

/// What a field of a sum constructor's payload holds: a scalar, or a sum by its constructors.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Scalar(Scalar),
    Sum(&'static [Payload]),
}

/// A constructor of a sum: its fixed id, the names of its fields and what each holds.
#[derive(Clone, Copy, Debug)]
pub struct Payload {
    id: CatCtorId,
    fields: Fields,
    kinds: &'static [Kind],
}

impl Payload {
    pub const fn fields(self) -> Fields {
        self.fields
    }
}

/// Returns constructor `id` of no fields.
pub const fn nullary(id: CatCtorId) -> Payload {
    Payload {
        id,
        fields: &[],
        kinds: &[],
    }
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

/// A field of a constructor whose type is a sum, with each of its constructors.
#[derive(Clone, Copy, Debug)]
pub struct Sum {
    field: Field,
    ctors: &'static [Payload],
}

/// A value of a sum field, each constructor at every level by its fixed id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Nested {
    Scalar(Value),
    /// A constructor and its fields' values, in the order its [`Payload`] lists them.
    Ctor {
        id: CatCtorId,
        fields: Fields,
        values: Vec<Nested>,
    },
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

/// The constructors of a sum field's type by declared position.
struct Table(Vec<Variant>);

struct Variant {
    payload: Payload,
    listed: Listed,
    /// For each field the payload lists, the table of its type when it holds a sum.
    kids: Vec<Option<Table>>,
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

    /// Returns `v`, a value of the table's type `fits` accepts, by its constructors' fixed ids.
    fn nested(&self, v: &Value) -> Nested {
        let Value::Sum { ctor, fields } = v else {
            panic!("decode checks a sum field holds a sum, not {v:?}");
        };
        let variant = &self.0[*ctor as usize];
        let values = variant
            .listed
            .handles()
            .zip(&variant.kids)
            .map(|(f, kid)| {
                let v = &fields[variant.listed.position(f)];
                match kid {
                    None => Nested::Scalar(v.clone()),
                    Some(t) => t.nested(v),
                }
            })
            .collect();
        Nested::Ctor {
            id: variant.payload.id,
            fields: variant.payload.fields,
            values,
        }
    }
}

impl Sum {
    pub fn field(self) -> Field {
        self.field
    }
}

/// Returns sum field `field`, whose type's constructors are `ctors` in any order.
pub const fn sum(field: Field, ctors: &'static [Payload]) -> Sum {
    Sum { field, ctors }
}

/// One constructor a codec reads: its fixed id, its fields, the sum fields it reads, and how a
/// row of it reads.
pub struct Ctor<T> {
    id: CatCtorId,
    fields: Fields,
    sums: &'static [Sum],
    read: fn(&Row<'_>) -> Result<T, Rot>,
}

impl<T> Ctor<T> {
    pub const fn new(id: CatCtorId, fields: Fields, read: fn(&Row<'_>) -> Result<T, Rot>) -> Self {
        Ctor {
            id,
            fields,
            sums: &[],
            read,
        }
    }

    /// Returns this constructor reading the sum fields `sums` by [`Row::ctor`].
    pub const fn with_sums(self, sums: &'static [Sum]) -> Self {
        Ctor { sums, ..self }
    }
}

/// A catalog relation a codec reads, by its marker type of [`relation`].
pub(crate) trait Relation: relation::Coding {
    /// The one type whose codec reads the relation.
    type Row;
}

macro_rules! coded {
    ($($variant:ident => $id:ident),* $(,)?) => {
        /// A catalog relation a codec reads.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Coded {
            $($variant),*
        }

        impl Coded {
            #[cfg(test)]
            pub const ALL: &'static [Coded] = &[$(Coded::$variant),*];

            /// Returns the fixed id of the relation.
            pub const fn id(self) -> CatRelId {
                match self {
                    $(Coded::$variant => engine_rel::$id),*
                }
            }
        }

        /// One marker type per relation a codec reads, each given its row type by one
        /// [`Relation`] impl.
        pub(crate) mod relation {
            use super::Coded;

            pub(crate) trait Coding {
                const CODED: Coded;
            }

            $(
                pub(crate) struct $variant;

                impl Coding for $variant {
                    const CODED: Coded = Coded::$variant;
                }
            )*
        }
    };
}

coded! {
    Roles => ROLES,
    Grants => GRANTS,
    Migrations => MIGRATIONS,
    Settings => SETTINGS,
    Bundles => BUNDLES,
    Registered => REGISTERED,
    Feeds => FEEDS,
    FeedMembers => FEED_MEMBERS,
    Credentials => CREDENTIALS,
    PeerMappings => PEER_MAPPINGS,
    Audit => AUDIT,
    VerifyFindings => VERIFY_FINDINGS,
    Types => TYPES,
    Constructors => CONSTRUCTORS,
    CtorStates => CTOR_STATES,
    ValueStates => VALUE_STATES,
    Fields => FIELDS,
    Relations => RELATIONS,
    NamedSets => NAMED_SETS,
    SetMembers => SET_MEMBERS,
    Indexes => INDEXES,
    IndexCtors => INDEX_CTORS,
    Constraints => CONSTRAINTS,
    Functions => FUNCTIONS,
    Defaults => DEFAULTS,
    Computed => COMPUTED,
    MigrationOps => MIGRATION_OPS,
    MigrationClauses => MIGRATION_CLAUSES,
}

/// A typed row of one catalog relation.
pub(crate) trait Decode: Sized + 'static {
    type Relation: Relation<Row = Self>;
    /// Each constructor of the relation.
    const CTORS: &'static [Ctor<Self>];
}

/// Returns the relation `T` reads.
pub const fn coded<T: Decode>() -> Coded {
    <T::Relation as relation::Coding>::CODED
}

/// The constructors and fields of the rows of the relation `T` reads in one schema.
pub struct Codec<T: 'static> {
    relation: RelId,
    name: String,
    ctors: Vec<Shape<T>>,
    row: PhantomData<fn() -> T>,
}

struct Shape<T: 'static> {
    ctor: &'static Ctor<T>,
    name: String,
    listed: Listed,
    /// For each field of `ctor.fields` that `ctor.sums` lists, the constructors of its type.
    sums: Vec<Option<Table>>,
    /// The stored type of each field of `ctor.fields`, in its order.
    types: Vec<FieldType>,
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

    pub fn relation(&self) -> RelId {
        self.relation
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

    fn table(&self, s: Sum) -> &Table {
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

    fn shape(&self, ctor: u32) -> Result<&Shape<T>, Rot> {
        self.ctors
            .get(ctor as usize)
            .ok_or_else(|| Rot::CatalogRow {
                row: self.name.clone(),
                detail: format!("has no constructor at position {ctor}"),
            })
    }

    /// Returns the typed row of `values`, a stored row of the constructor at position `ctor`.
    ///
    /// Fail with [`Rot::CatalogRow`] when the relation has no constructor at `ctor`, `values`
    /// holds another count of fields than its constructor, a field holds a value not of its
    /// declared type, or the constructor's reader refuses a value.
    pub fn decode(&self, ctor: u32, values: &[Value]) -> Result<T, Rot> {
        let shape = self.shape(ctor)?;
        let row = Row {
            relation: &self.name,
            name: &shape.name,
            ctor: shape.ctor.id,
            fields: shape.ctor.fields,
            positions: &shape.listed.positions,
            sums: &shape.sums,
            values,
        };
        if values.len() != shape.listed.positions.len() {
            return Err(row.refused(counted(values.len(), shape.listed.positions.len())));
        }
        for (at, t) in shape.types.iter().enumerate() {
            let f = Field {
                fields: shape.ctor.fields,
                at,
            };
            if !fits(t, row.get(f)) {
                return Err(row.bad(f));
            }
        }
        (shape.ctor.read)(&row)
    }

    /// Returns the typed row of `v`, a value of the relation's element type.
    ///
    /// Fail as [`Codec::decode`] does, and when `v` is not a constructor's value, holds another
    /// count of fields than its constructor, or a field of it has no stored form of its type.
    pub fn decode_val(&self, schema: &Schema, v: &Val) -> Result<T, Rot> {
        let Val::Ctor(c, fields) = v else {
            return Err(Rot::CatalogRow {
                row: self.name.clone(),
                detail: format!("is {v:?}"),
            });
        };
        let shape = self.shape(*c)?;
        let declared = layout::row(schema, self.relation, *c);
        if fields.len() != declared.len() {
            return Err(Rot::CatalogRow {
                row: format!("{}.{}", self.name, shape.name),
                detail: counted(fields.len(), declared.len()),
            });
        }
        let stored = declared
            .iter()
            .zip(fields.iter())
            .map(|((_, t), v)| exec::to_stored(&layout::field_type(schema, t), v).ok())
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| Rot::CatalogRow {
                row: self.name.clone(),
                detail: format!("has no stored form: {v:?}"),
            })?;
        self.decode(*c, &stored)
    }
}

/// Returns the detail of a row holding `held` fields where its constructor declares `declared`.
fn counted(held: usize, declared: usize) -> String {
    let fields = if held == 1 { "field" } else { "fields" };
    format!("holds {held} {fields}, not {declared}")
}

/// Returns whether `v` is a value of stored type `t`.
fn fits(t: &FieldType, v: &Value) -> bool {
    match (t, v) {
        (FieldType::Scalar(s), v) => scalar_fits(*s, v),
        (FieldType::Record(ts), Value::Record(vs)) => all_fit(ts, vs),
        (FieldType::Sum(cs), Value::Sum { ctor, fields }) => cs
            .get(*ctor as usize)
            .is_some_and(|c| all_fit(&c.fields, fields)),
        _ => false,
    }
}

fn all_fit(ts: &[FieldType], vs: &[Value]) -> bool {
    ts.len() == vs.len() && ts.iter().zip(vs).all(|(t, v)| fits(t, v))
}

fn scalar_fits(s: Scalar, v: &Value) -> bool {
    matches!(
        (s, v),
        (Scalar::I8, Value::I8(_))
            | (Scalar::I16, Value::I16(_))
            | (Scalar::I32, Value::I32(_))
            | (Scalar::I64, Value::I64(_))
            | (Scalar::F32, Value::F32(_))
            | (Scalar::F64, Value::F64(_))
            | (Scalar::Decimal(_), Value::Decimal(_))
            | (Scalar::Text, Value::Text(_))
            | (Scalar::Bytes, Value::Bytes(_))
            | (Scalar::SortKey, Value::SortKey(_))
            | (Scalar::Instant, Value::Instant(_))
            | (Scalar::Date, Value::Date(_))
            | (Scalar::Time, Value::Time(_))
            | (Scalar::DateTime, Value::DateTime(_))
            | (Scalar::ZonedDateTime, Value::ZonedDateTime { .. })
            | (Scalar::Duration, Value::Duration(_))
            | (Scalar::Period, Value::Period { .. })
    )
}

/// One stored row of a catalog relation, its fields read by [`Field`].
pub struct Row<'a> {
    relation: &'a str,
    name: &'a str,
    ctor: CatCtorId,
    fields: Fields,
    positions: &'a [usize],
    sums: &'a [Option<Table>],
    values: &'a [Value],
}

impl Row<'_> {
    /// Returns the refusal of this row as damage, `detail` saying why.
    pub fn refused(&self, detail: String) -> Rot {
        Rot::CatalogRow {
            row: format!("{}.{}", self.relation, self.name),
            detail,
        }
    }

    fn get(&self, f: Field) -> &Value {
        assert!(
            f.fields == self.fields,
            "a codec reads a field of the constructor it reads"
        );
        &self.values[self.positions[f.at]]
    }

    /// Returns the fixed id of the row's constructor.
    pub fn ctor_id(&self) -> CatCtorId {
        self.ctor
    }

    /// Returns the refusal of the value field `f` holds.
    pub fn bad(&self, f: Field) -> Rot {
        self.refused(format!(
            "holds {:?} in field `{}`",
            self.get(f),
            f.fields[f.at]
        ))
    }

    /// Abort on field `f` holding another type than its reader takes, which `decode` refuses first.
    fn unchecked(&self, f: Field) -> ! {
        panic!(
            "decode checks field `{}` of {}.{} against its stored type before a reader reads it",
            f.fields[f.at], self.relation, self.name
        )
    }

    pub fn int(&self, f: Field) -> i64 {
        match self.get(f) {
            Value::I64(n) => *n,
            _ => self.unchecked(f),
        }
    }

    /// Returns the integer in field `f`, an `I32` widened.
    pub fn int32(&self, f: Field) -> i64 {
        match self.get(f) {
            Value::I32(n) => i64::from(*n),
            _ => self.unchecked(f),
        }
    }

    pub fn bytes(&self, f: Field) -> &[u8] {
        match self.get(f) {
            Value::Bytes(b) => b,
            _ => self.unchecked(f),
        }
    }

    /// Returns the truth of the `Bool` in field `f`.
    pub fn flag(&self, f: Field) -> bool {
        match self.get(f) {
            Value::Sum { ctor, .. } => *ctor == ctor::TRUE,
            _ => self.unchecked(f),
        }
    }

    /// Returns the fixed id of the constructor sum field `s` holds, with that constructor's
    /// fields, of the types `s` lists for it.
    pub fn ctor(&self, s: Sum) -> (CatCtorId, &[Value]) {
        match self.get(s.field) {
            Value::Sum { ctor, fields } => (
                self.table(s).0[*ctor as usize].payload.id,
                fields.as_slice(),
            ),
            _ => self.unchecked(s.field),
        }
    }

    /// Returns the value sum field `s` holds, by the fixed ids of its constructors.
    pub fn nested(&self, s: Sum) -> Nested {
        self.table(s).nested(self.get(s.field))
    }

    fn table(&self, s: Sum) -> &Table {
        self.sums[s.field.at]
            .as_ref()
            .expect("a codec reads a sum field its constructor lists")
    }

    pub fn text(&self, f: Field) -> &str {
        match self.get(f) {
            Value::Text(t) => t,
            _ => self.unchecked(f),
        }
    }

    pub fn instant(&self, f: Field) -> i64 {
        match self.get(f) {
            Value::Instant(t) => *t,
            _ => self.unchecked(f),
        }
    }

    /// Returns the duration in field `f`, in microseconds.
    pub fn duration(&self, f: Field) -> i64 {
        match self.get(f) {
            Value::Duration(d) => *d,
            _ => self.unchecked(f),
        }
    }

    pub fn float(&self, f: Field) -> F64 {
        match self.get(f) {
            Value::F64(x) => *x,
            _ => self.unchecked(f),
        }
    }

    /// Returns the count in `I64` field `f`, refusing one below 0 as out of range.
    pub fn count(&self, f: Field) -> Result<NonNegative, Rot> {
        NonNegative::new(self.int(f)).ok_or_else(|| self.out_of_range(f))
    }

    /// Returns the duration in field `f`, refusing one below 0 as out of range.
    pub fn length(&self, f: Field) -> Result<NonNegative, Rot> {
        NonNegative::new(self.duration(f)).ok_or_else(|| self.out_of_range(f))
    }

    /// Returns the duration an `Option Duration` field `f` holds, refusing one below 0 as out of
    /// range.
    pub fn optional_length(&self, f: Field) -> Result<Option<NonNegative>, Rot> {
        self.optional_duration(f)
            .map(|d| NonNegative::new(d).ok_or_else(|| self.out_of_range(f)))
            .transpose()
    }

    /// Returns the fraction in `F64` field `f`, refusing one outside 0 to 1 as out of range.
    pub fn fraction(&self, f: Field) -> Result<F64, Rot> {
        let x = self.float(f);
        if (0.0..=1.0).contains(&x.get()) {
            Ok(x)
        } else {
            Err(self.out_of_range(f))
        }
    }

    fn out_of_range(&self, f: Field) -> Rot {
        Rot::OutOfRange {
            row: format!("{}.{}", self.relation, self.name),
            field: f.fields[f.at],
        }
    }

    /// Returns the duration an `Option Duration` field holds, in microseconds.
    pub fn optional_duration(&self, f: Field) -> Option<i64> {
        self.optional(f, |v| match v {
            Value::Duration(d) => Some(*d),
            _ => None,
        })
    }

    /// Returns the integer an `Option I64` field holds.
    pub fn optional_int(&self, f: Field) -> Option<i64> {
        self.optional(f, |v| match v {
            Value::I64(n) => Some(*n),
            _ => None,
        })
    }

    /// Returns the text an `Option Text` field holds.
    pub fn optional_text(&self, f: Field) -> Option<&str> {
        self.optional(f, |v| match v {
            Value::Text(t) => Some(t.as_str()),
            _ => None,
        })
    }

    fn optional<'v, T>(&'v self, f: Field, inner: impl Fn(&'v Value) -> Option<T>) -> Option<T> {
        match self.get(f) {
            Value::Sum {
                ctor: ctor::SOME,
                fields,
            } => Some(inner(&fields[0]).unwrap_or_else(|| self.unchecked(f))),
            Value::Sum { .. } => None,
            _ => self.unchecked(f),
        }
    }
}

/// Returns the instant of `payload`, the fields of a sum constructor whose [`Sum`] lists them as
/// one `Instant`.
pub fn payload_instant(payload: &[Value]) -> i64 {
    match payload {
        [Value::Instant(t)] => *t,
        _ => {
            panic!("Codec::new checks a sum constructor's payload against the types its Sum lists")
        }
    }
}

/// Abort on nested constructor `id` its `Sum` does not list, which `Codec::new` rules out.
pub fn unlisted(id: CatCtorId) -> ! {
    panic!("Codec::new checks constructor {id} is listed by its Sum")
}

/// A row of `catalog::roles`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleRow {
    id: i64,
    name: String,
    created: i64,
}

impl RoleRow {
    /// Returns the role's id, name and creation instant.
    pub fn into_parts(self) -> (i64, String, i64) {
        (self.id, self.name, self.created)
    }
}

const ROLE: Fields = &["id", "name", "created"];
pub const ROLE_ID: Field = field(ROLE, "id");
pub const ROLE_NAME: Field = field(ROLE, "name");
pub const ROLE_CREATED: Field = field(ROLE, "created");

impl Relation for relation::Roles {
    type Row = RoleRow;
}

impl Decode for RoleRow {
    type Relation = relation::Roles;
    const CTORS: &'static [Ctor<RoleRow>] = &[Ctor::new(k::ROLE, ROLE, |row| {
        Ok(RoleRow {
            id: row.int(ROLE_ID),
            name: row.text(ROLE_NAME).to_owned(),
            created: row.instant(ROLE_CREATED),
        })
    })];
}

/// Returns the privilege a row of `catalog::grants` names `name`.
///
/// Fail with [`Rot::CatalogRow`] when `name` names none.
fn privilege(name: &str) -> Result<Privilege, Rot> {
    Privilege::named(name).ok_or_else(|| Rot::CatalogRow {
        row: "catalog::grants".into(),
        detail: format!("names no privilege `{name}`"),
    })
}

/// What wrote a row of `catalog::grants`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantKind {
    Log,
    Deployment,
}

/// A row of `catalog::grants`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleGrant {
    id: i64,
    kind: GrantKind,
    privilege: Privilege,
    target: Option<String>,
    grantee: String,
    grantor: String,
    grant_option: bool,
    dormant: bool,
}

impl RoleGrant {
    pub fn id(&self) -> i64 {
        self.id
    }

    pub fn kind(&self) -> GrantKind {
        self.kind
    }

    /// Returns the fields of the row that decide what its grant option supports.
    pub fn row(&self) -> GrantRow<'_> {
        GrantRow {
            grantor: &self.grantor,
            grantee: &self.grantee,
            privilege: self.privilege,
            target: self.target.as_deref(),
            grant_option: self.grant_option,
            dormant: self.dormant,
        }
    }

    /// Returns what the row gives its grantee.
    pub fn into_deployed(self) -> DeployedGrant {
        DeployedGrant {
            grantee: self.grantee,
            privilege: self.privilege,
            target: self.target,
            grant_option: self.grant_option,
            dormant: self.dormant,
        }
    }

    /// Returns whether `other` differs from this row only in id and dormancy.
    pub fn pairs(&self, other: &RoleGrant) -> bool {
        RoleGrant {
            id: other.id,
            dormant: other.dormant,
            ..self.clone()
        } == *other
    }

    /// Returns `v`, a row of `codec`'s relation it reads, with id `id`.
    pub fn keyed(codec: &Codec<RoleGrant>, v: &Val, id: i64) -> Val {
        codec.with(v, GRANT_ID, Val::Int(id))
    }
}

const GRANT: Fields = &[
    "id",
    "kind",
    "privilege",
    "target",
    "grantee",
    "grantor",
    "grant_option",
    "dormant",
];
pub const GRANT_ID: Field = field(GRANT, "id");
pub const GRANT_KIND: Sum = sum(
    field(GRANT, "kind"),
    &[nullary(k::GRANT_LOG), nullary(k::GRANT_DEPLOYMENT)],
);
pub const GRANT_PRIVILEGE: Field = field(GRANT, "privilege");
pub const GRANT_TARGET: Field = field(GRANT, "target");
pub const GRANT_GRANTEE: Field = field(GRANT, "grantee");
pub const GRANT_GRANTOR: Field = field(GRANT, "grantor");
pub const GRANT_OPTION: Field = field(GRANT, "grant_option");
pub const GRANT_DORMANT: Field = field(GRANT, "dormant");

impl Relation for relation::Grants {
    type Row = RoleGrant;
}

impl Decode for RoleGrant {
    type Relation = relation::Grants;
    const CTORS: &'static [Ctor<RoleGrant>] = &[Ctor::new(k::ROLE_GRANT, GRANT, |r| {
        Ok(RoleGrant {
            id: r.int(GRANT_ID),
            kind: match r.ctor(GRANT_KIND).0 {
                k::GRANT_LOG => GrantKind::Log,
                k::GRANT_DEPLOYMENT => GrantKind::Deployment,
                other => unlisted(other),
            },
            privilege: privilege(r.text(GRANT_PRIVILEGE))?,
            target: r.optional_text(GRANT_TARGET).map(str::to_owned),
            grantee: r.text(GRANT_GRANTEE).to_owned(),
            grantor: r.text(GRANT_GRANTOR).to_owned(),
            grant_option: r.flag(GRANT_OPTION),
            dormant: r.flag(GRANT_DORMANT),
        })
    })
    .with_sums(&[GRANT_KIND])];
}

/// A row of `catalog::migrations`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationRow {
    version: u64,
    hash: Hash,
    applied: i64,
}

impl MigrationRow {
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn hash(&self) -> &Hash {
        &self.hash
    }

    /// Returns the instant the migration was applied, in microseconds.
    pub fn applied(&self) -> i64 {
        self.applied
    }
}

const MIGRATION_ROW: Fields = &["version", "hash", "text", "applied"];
pub const MIGRATION_VERSION: Field = field(MIGRATION_ROW, "version");
pub const MIGRATION_HASH: Field = field(MIGRATION_ROW, "hash");
pub const MIGRATION_TEXT: Field = field(MIGRATION_ROW, "text");
pub const MIGRATION_APPLIED: Field = field(MIGRATION_ROW, "applied");

impl Relation for relation::Migrations {
    type Row = MigrationRow;
}

impl Decode for MigrationRow {
    type Relation = relation::Migrations;
    const CTORS: &'static [Ctor<MigrationRow>] = &[Ctor::new(k::MIGRATION, MIGRATION_ROW, |r| {
        let hash = r.bytes(MIGRATION_HASH);
        Ok(MigrationRow {
            version: u64::try_from(r.int(MIGRATION_VERSION))
                .map_err(|_| r.bad(MIGRATION_VERSION))?,
            hash: hash
                .try_into()
                .map_err(|_| r.refused(format!("holds a hash of {} bytes, not 32", hash.len())))?,
            applied: r.instant(MIGRATION_APPLIED),
        })
    })];
}

/// A count or duration of a setting, in microseconds for a duration: an `i64` from 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NonNegative(i64);

impl NonNegative {
    /// Returns `n`, `None` when it is below 0.
    pub fn new(n: i64) -> Option<NonNegative> {
        (n >= 0).then_some(NonNegative(n))
    }

    pub fn get(self) -> i64 {
        self.0
    }

    pub fn unsigned(self) -> u64 {
        self.0.cast_unsigned()
    }
}

/// A row of `catalog::settings`. Each fraction of `VacuumBudget` is from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingRow {
    StatementLimit(Option<NonNegative>),
    TransactionLimit(Option<NonNegative>),
    IdleLimit(Option<NonNegative>),
    StreamIdleLimit(NonNegative),
    RequestWindow(NonNegative),
    SafePointDelay(NonNegative),
    ReplicaWait(NonNegative),
    DefaultLease(NonNegative),
    FeedRetention(NonNegative),
    StatsWindow(NonNegative),
    StatsRetention(NonNegative),
    WaitSampling {
        every: NonNegative,
        keep: NonNegative,
    },
    ConflictLog(NonNegative),
    VacuumBudget {
        io: F64,
        cpu: F64,
    },
    ScrubInterval(Option<NonNegative>),
    LeaseGrace(NonNegative),
    BlockingNoticePeriod(NonNegative),
    ReplicaVacuumWait(NonNegative),
    RetryLimit(NonNegative),
    RetentionWindow(NonNegative),
}

const LIMIT: Fields = &["id", "limit"];
const KEEP: Fields = &["id", "keep"];
const DELAY: Fields = &["id", "delay"];
const WAIT: Fields = &["id", "wait"];
const LEASE: Fields = &["id", "lease"];
const LENGTH: Fields = &["id", "length"];
const SAMPLING: Fields = &["id", "every", "keep"];
const ROWS: Fields = &["id", "rows"];
const BUDGET: Fields = &["id", "io", "cpu"];
const EVERY: Fields = &["id", "every"];
const GRACE: Fields = &["id", "grace"];
const PERIOD: Fields = &["id", "period"];
const RETRIES: Fields = &["id", "retries"];

const LIMIT_F: Field = field(LIMIT, "limit");
const KEEP_F: Field = field(KEEP, "keep");
const DELAY_F: Field = field(DELAY, "delay");
const WAIT_F: Field = field(WAIT, "wait");
const LEASE_F: Field = field(LEASE, "lease");
const LENGTH_F: Field = field(LENGTH, "length");
const SAMPLING_EVERY: Field = field(SAMPLING, "every");
const SAMPLING_KEEP: Field = field(SAMPLING, "keep");
const ROWS_F: Field = field(ROWS, "rows");
const IO: Field = field(BUDGET, "io");
const CPU: Field = field(BUDGET, "cpu");
const EVERY_F: Field = field(EVERY, "every");
const GRACE_F: Field = field(GRACE, "grace");
const PERIOD_F: Field = field(PERIOD, "period");
const RETRIES_F: Field = field(RETRIES, "retries");

const BUDGET_ID: Field = field(BUDGET, "id");
const DELAY_ID: Field = field(DELAY, "id");
const EVERY_ID: Field = field(EVERY, "id");
const GRACE_ID: Field = field(GRACE, "id");
const KEEP_ID: Field = field(KEEP, "id");
const LEASE_ID: Field = field(LEASE, "id");
const LENGTH_ID: Field = field(LENGTH, "id");
const LIMIT_ID: Field = field(LIMIT, "id");
const PERIOD_ID: Field = field(PERIOD, "id");
const RETRIES_ID: Field = field(RETRIES, "id");
const ROWS_ID: Field = field(ROWS, "id");
const SAMPLING_ID: Field = field(SAMPLING, "id");
const WAIT_ID: Field = field(WAIT, "id");

/// The header field `id` of each list of setting fields.
const SETTING_IDS: &[Field] = &[
    BUDGET_ID,
    DELAY_ID,
    EVERY_ID,
    GRACE_ID,
    KEEP_ID,
    LEASE_ID,
    LENGTH_ID,
    LIMIT_ID,
    PERIOD_ID,
    RETRIES_ID,
    ROWS_ID,
    SAMPLING_ID,
    WAIT_ID,
];

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

/// Returns `setting`, once the header field `id` of `r` holds its constructor's catalog id.
fn keyed(r: &Row<'_>, id: Field, setting: SettingRow) -> Result<SettingRow, Rot> {
    if u64::try_from(r.int(id)) == Ok(r.ctor_id().raw()) {
        Ok(setting)
    } else {
        Err(r.bad(id))
    }
}

impl Relation for relation::Settings {
    type Row = SettingRow;
}

impl Decode for SettingRow {
    type Relation = relation::Settings;
    const CTORS: &'static [Ctor<SettingRow>] = {
        use SettingRow as S;
        &[
            Ctor::new(k::SETTING_STATEMENT_LIMIT, LIMIT, |r| {
                keyed(r, LIMIT_ID, S::StatementLimit(r.optional_length(LIMIT_F)?))
            }),
            Ctor::new(k::SETTING_TRANSACTION_LIMIT, LIMIT, |r| {
                keyed(
                    r,
                    LIMIT_ID,
                    S::TransactionLimit(r.optional_length(LIMIT_F)?),
                )
            }),
            Ctor::new(k::SETTING_IDLE_LIMIT, LIMIT, |r| {
                keyed(r, LIMIT_ID, S::IdleLimit(r.optional_length(LIMIT_F)?))
            }),
            Ctor::new(k::SETTING_STREAM_IDLE_LIMIT, LIMIT, |r| {
                keyed(r, LIMIT_ID, S::StreamIdleLimit(r.length(LIMIT_F)?))
            }),
            Ctor::new(k::SETTING_REQUEST_WINDOW, KEEP, |r| {
                keyed(r, KEEP_ID, S::RequestWindow(r.length(KEEP_F)?))
            }),
            Ctor::new(k::SETTING_SAFE_POINT_DELAY, DELAY, |r| {
                keyed(r, DELAY_ID, S::SafePointDelay(r.length(DELAY_F)?))
            }),
            Ctor::new(k::SETTING_REPLICA_WAIT, WAIT, |r| {
                keyed(r, WAIT_ID, S::ReplicaWait(r.length(WAIT_F)?))
            }),
            Ctor::new(k::SETTING_DEFAULT_LEASE, LEASE, |r| {
                keyed(r, LEASE_ID, S::DefaultLease(r.length(LEASE_F)?))
            }),
            Ctor::new(k::SETTING_FEED_RETENTION, KEEP, |r| {
                keyed(r, KEEP_ID, S::FeedRetention(r.length(KEEP_F)?))
            }),
            Ctor::new(k::SETTING_STATS_WINDOW, LENGTH, |r| {
                keyed(r, LENGTH_ID, S::StatsWindow(r.length(LENGTH_F)?))
            }),
            Ctor::new(k::SETTING_STATS_RETENTION, KEEP, |r| {
                keyed(r, KEEP_ID, S::StatsRetention(r.length(KEEP_F)?))
            }),
            Ctor::new(k::SETTING_WAIT_SAMPLING, SAMPLING, |r| {
                keyed(
                    r,
                    SAMPLING_ID,
                    S::WaitSampling {
                        every: r.length(SAMPLING_EVERY)?,
                        keep: r.length(SAMPLING_KEEP)?,
                    },
                )
            }),
            Ctor::new(k::SETTING_CONFLICT_LOG, ROWS, |r| {
                keyed(r, ROWS_ID, S::ConflictLog(r.count(ROWS_F)?))
            }),
            Ctor::new(k::SETTING_VACUUM_BUDGET, BUDGET, |r| {
                keyed(
                    r,
                    BUDGET_ID,
                    S::VacuumBudget {
                        io: r.fraction(IO)?,
                        cpu: r.fraction(CPU)?,
                    },
                )
            }),
            Ctor::new(k::SETTING_SCRUB_INTERVAL, EVERY, |r| {
                keyed(r, EVERY_ID, S::ScrubInterval(r.optional_length(EVERY_F)?))
            }),
            Ctor::new(k::SETTING_LEASE_GRACE, GRACE, |r| {
                keyed(r, GRACE_ID, S::LeaseGrace(r.length(GRACE_F)?))
            }),
            Ctor::new(k::SETTING_BLOCKING_NOTICE_PERIOD, PERIOD, |r| {
                keyed(r, PERIOD_ID, S::BlockingNoticePeriod(r.length(PERIOD_F)?))
            }),
            Ctor::new(k::SETTING_REPLICA_VACUUM_WAIT, WAIT, |r| {
                keyed(r, WAIT_ID, S::ReplicaVacuumWait(r.length(WAIT_F)?))
            }),
            Ctor::new(k::SETTING_RETRY_LIMIT, RETRIES, |r| {
                keyed(r, RETRIES_ID, S::RetryLimit(r.count(RETRIES_F)?))
            }),
            Ctor::new(k::SETTING_RETENTION_WINDOW, LENGTH, |r| {
                keyed(r, LENGTH_ID, S::RetentionWindow(r.length(LENGTH_F)?))
            }),
        ]
    };
}
