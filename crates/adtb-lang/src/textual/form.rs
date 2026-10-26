use crate::schema::{Generator, Schema, TypeDefKind};
use crate::types::{Base, CtorSet, RelId, Ty};

/// The shape of the values of a type, as their textual syntax needs it.
#[derive(Clone, Debug, PartialEq)]
pub enum Form {
    Base(Base),
    Uuid,
    /// A newtype other than `Uuid`, by name.
    Newtype {
        name: String,
        base: Box<Form>,
    },
    /// A `Span`, by the form of its points.
    Span(Box<Form>),
    /// A sum's constructors in index order, and the constructors a value may take when the type
    /// restricts them.
    Sum {
        ctors: Vec<Ctor>,
        allowed: Option<CtorSet>,
    },
    /// The names of the constructors of a tag's sum, in index order.
    Tag(Vec<String>),
    /// An anonymous record, its fields sorted by name.
    Record(Vec<(String, Form)>),
    Unit,
    Key {
        rel: String,
        value: KeyForm,
    },
    Set {
        elem: Box<Form>,
        nonempty: bool,
    },
    /// A type with no textual syntax: a type parameter, a function, a group or a change.
    Opaque,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ctor {
    pub name: String,
    /// The fields, header fields first, each in declaration order.
    pub fields: Vec<(String, Form)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum KeyForm {
    One(Box<Form>),
    /// The fields of a composite key, sorted by name.
    Fields(Vec<(String, Form)>),
}

/// Returns whether key field `f` of `rel` is a `Key` of `rel` itself.
pub fn self_key(schema: &Schema, rel: RelId, f: &str) -> bool {
    let r = schema.relation(rel);
    matches!(
        schema.ty(r.elem_def).field(0, f).map(|d| &d.ty),
        Some(Ty::Key { rel: own, .. }) if *own == rel
    )
}

/// Returns the type of key field `f` of `rel`, where a key field that is a key of `rel` itself
/// has the type of the value its generator draws.
pub fn key_field_ty(schema: &Schema, rel: RelId, f: &str) -> Ty {
    let r = schema.relation(rel);
    if self_key(schema, rel, f) {
        return match r.generate {
            Some(Generator::Uuidv7) => Ty::Data {
                def: schema.prelude_type("Uuid"),
                args: Vec::new(),
                ctors: None,
            },
            _ => Ty::Base(Base::I64),
        };
    }
    schema
        .ty(r.elem_def)
        .field(0, f)
        .unwrap_or_else(|| panic!("`{}` has key field `{f}`", r.name))
        .ty
        .subst(r.elem_args())
}

fn fields_of<'a>(
    schema: &Schema,
    fs: impl IntoIterator<Item = &'a (String, Ty)>,
) -> Vec<(String, Form)> {
    fs.into_iter()
        .map(|(n, t)| (n.clone(), Form::of(schema, t)))
        .collect()
}

impl Form {
    /// Returns the form of the values of `ty`, a type of `schema`.
    pub fn of(schema: &Schema, ty: &Ty) -> Form {
        match ty {
            Ty::Base(b) => Form::Base(*b),
            Ty::Data { def, args, ctors } => {
                let d = schema.ty(*def);
                match &d.kind {
                    TypeDefKind::Newtype { .. } if *def == schema.prelude_type("Uuid") => {
                        Form::Uuid
                    }
                    TypeDefKind::Newtype { base, .. } => Form::Newtype {
                        name: d.name.clone(),
                        base: Box::new(Form::of(schema, &base.subst(args))),
                    },
                    TypeDefKind::Sum { .. } if *def == schema.prelude_type("Span") => {
                        Form::Span(Box::new(Form::of(schema, &args[0])))
                    }
                    TypeDefKind::Sum {
                        header, ctors: all, ..
                    } => Form::Sum {
                        ctors: all
                            .iter()
                            .map(|c| Ctor {
                                name: c.name.clone(),
                                fields: header
                                    .iter()
                                    .chain(&c.fields)
                                    .map(|f| (f.name.clone(), Form::of(schema, &f.ty.subst(args))))
                                    .collect(),
                            })
                            .collect(),
                        allowed: ctors.clone(),
                    },
                }
            }
            Ty::Sum { ctors, sel } => Form::Sum {
                ctors: ctors
                    .iter()
                    .map(|(name, fs)| Ctor {
                        name: name.clone(),
                        fields: fields_of(schema, fs),
                    })
                    .collect(),
                allowed: sel.clone(),
            },
            Ty::Tag(inner) => match inner.as_ref() {
                Ty::Data { def, .. } => Form::Tag(
                    schema
                        .ty(*def)
                        .ctors()
                        .iter()
                        .map(|c| c.name.clone())
                        .collect(),
                ),
                Ty::Sum { ctors, .. } => Form::Tag(ctors.iter().map(|(n, _)| n.clone()).collect()),
                _ => Form::Opaque,
            },
            Ty::Record(fs) => Form::Record(fields_of(schema, fs)),
            Ty::Unit => Form::Unit,
            Ty::Key { rel, .. } => {
                let r = schema.relation(*rel);
                let value = match r.key.as_slice() {
                    [f] => KeyForm::One(Box::new(Form::of(schema, &key_field_ty(schema, *rel, f)))),
                    fs => {
                        let mut names: Vec<&String> = fs.iter().collect();
                        names.sort();
                        KeyForm::Fields(
                            names
                                .into_iter()
                                .map(|f| {
                                    (f.clone(), Form::of(schema, &key_field_ty(schema, *rel, f)))
                                })
                                .collect(),
                        )
                    }
                };
                Form::Key {
                    rel: r.name.clone(),
                    value,
                }
            }
            Ty::Set(t) => Form::Set {
                elem: Box::new(Form::of(schema, t)),
                nonempty: false,
            },
            Ty::NonEmpty(t) => Form::Set {
                elem: Box::new(Form::of(schema, t)),
                nonempty: true,
            },
            Ty::Param(_) | Ty::Group(..) | Ty::Fn(..) | Ty::Change => Form::Opaque,
        }
    }

    /// Returns whether every value of the form has a textual syntax.
    pub fn has_syntax(&self) -> bool {
        match self {
            Form::Opaque => false,
            Form::Base(_) | Form::Uuid | Form::Tag(_) | Form::Unit => true,
            Form::Newtype { base: f, .. } | Form::Span(f) | Form::Set { elem: f, .. } => {
                f.has_syntax()
            }
            Form::Key {
                value: KeyForm::One(f),
                ..
            } => f.has_syntax(),
            Form::Record(fs)
            | Form::Key {
                value: KeyForm::Fields(fs),
                ..
            } => fs.iter().all(|(_, f)| f.has_syntax()),
            Form::Sum { ctors, .. } => ctors
                .iter()
                .all(|c| c.fields.iter().all(|(_, f)| f.has_syntax())),
        }
    }
}
