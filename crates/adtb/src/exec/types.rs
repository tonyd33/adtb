use adtb_lang::schema::{Schema, TypeDefKind};
use adtb_lang::types::{Base, RelId, Ty};

use super::Error;

/// Returns `ty` with every newtype at its top replaced by its representation.
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

/// The constructors a value of a sum type is built from, each with its fields in the order a
/// `Val::Ctor` holds them.
pub fn ctors(schema: &Schema, ty: &Ty) -> Option<Vec<Vec<(String, Ty)>>> {
    match repr(schema, ty) {
        Ty::Data { def, args, .. } => {
            let d = schema.ty(def);
            let header: Vec<(String, Ty)> = d
                .header()
                .iter()
                .map(|f| (f.name.clone(), f.ty.subst(&args)))
                .collect();
            Some(
                d.ctors()
                    .iter()
                    .map(|c| {
                        let mut fs = header.clone();
                        fs.extend(c.fields.iter().map(|f| (f.name.clone(), f.ty.subst(&args))));
                        fs
                    })
                    .collect(),
            )
        }
        Ty::Sum { ctors, .. } => Some(ctors.into_iter().map(|(_, fs)| fs).collect()),
        Ty::Tag(t) => Some(ctors(schema, &t)?.into_iter().map(|_| Vec::new()).collect()),
        _ => None,
    }
}

/// The constructors a value of `ty` may hold: its constructor set, or every constructor.
pub fn ctor_set(schema: &Schema, ty: &Ty) -> Vec<u32> {
    let all = || (0..ctors(schema, ty).map_or(0, |c| c.len()) as u32).collect();
    match repr(schema, ty) {
        Ty::Data { ctors: Some(s), .. } | Ty::Sum { sel: Some(s), .. } => s,
        Ty::Tag(t) => ctor_set(schema, &t),
        _ => all(),
    }
}

/// Where a field is in the values of a type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum At {
    /// At this position whatever the constructor.
    Fixed(usize),
    /// At the position this constructor's entry names; `None` for a constructor the type
    /// excludes.
    ByCtor(Vec<Option<usize>>),
}

impl At {
    pub fn of(&self, ctor: u32) -> usize {
        match self {
            At::Fixed(i) => *i,
            At::ByCtor(at) => at[ctor as usize].expect("a constructor of the type has the field"),
        }
    }
}

/// Returns where field `name` of a value of type `ty` lies, and the field's type.
pub fn field(schema: &Schema, ty: &Ty, name: &str) -> Result<(At, Ty), Error> {
    if let Ty::Record(fs) = repr(schema, ty) {
        let i = fs
            .iter()
            .position(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("a record of type {ty:?} has field `{name}`"));
        return Ok((At::Fixed(i), fs[i].1.clone()));
    }
    let cs = ctors(schema, ty)
        .ok_or_else(|| Error::Unsupported(format!("field `{name}` of a value of type {ty:?}")))?;
    let set = ctor_set(schema, ty);
    let mut at = vec![None; cs.len()];
    let mut field_ty = None;
    for &c in &set {
        let i = cs[c as usize]
            .iter()
            .position(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("constructor {c} of {ty:?} has field `{name}`"));
        at[c as usize] = Some(i);
        field_ty.get_or_insert_with(|| cs[c as usize][i].1.clone());
    }
    let first = set.first().and_then(|&c| at[c as usize]);
    let fixed = set.iter().all(|&c| at[c as usize] == first);
    let ty = field_ty.expect("a row type names a constructor");
    match (fixed, first) {
        (true, Some(i)) => Ok((At::Fixed(i), ty)),
        _ => Ok((At::ByCtor(at), ty)),
    }
}

/// Returns, for a row of relation `rel` of type `row`, where each key field is, in name order,
/// and whether the key is one field's value rather than a record of them.
pub fn key_fields(schema: &Schema, rel: RelId, row: &Ty) -> Result<(Vec<At>, bool), Error> {
    let mut names = schema.relation(rel).key.clone();
    let single = names.len() == 1;
    names.sort();
    let at = names
        .iter()
        .map(|n| field(schema, row, n).map(|(a, _)| a))
        .collect::<Result<_, _>>()?;
    Ok((at, single))
}

/// Returns the base type `ty` represents, if it represents one.
pub fn base(schema: &Schema, ty: &Ty) -> Option<Base> {
    match repr(schema, ty) {
        Ty::Base(b) => Some(b),
        _ => None,
    }
}

/// Returns the type an `Option` type holds.
pub fn option_arg(schema: &Schema, ty: &Ty) -> Option<Ty> {
    match repr(schema, ty) {
        Ty::Data { def, args, .. } if def == schema.prelude_type("Option") => args.first().cloned(),
        _ => None,
    }
}

/// Returns the element type of a set type.
pub fn elem(schema: &Schema, ty: &Ty) -> Option<Ty> {
    match repr(schema, ty) {
        Ty::Set(t) | Ty::NonEmpty(t) => Some(*t),
        _ => None,
    }
}
