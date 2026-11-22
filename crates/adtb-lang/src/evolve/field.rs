use super::*;

impl Cx<'_> {
    /// The row type a fill or routing for `part` reads: the whole sum for the header.
    pub(super) fn row_ty(schema: &Schema, def: TypeId, ctors: Option<Vec<u32>>) -> Ty {
        let t = schema.ty(def);
        Ty::Data {
            def,
            args: (0..t.params.len() as u32).map(Ty::Param).collect(),
            ctors: ctors.and_then(|cs| schema.normalize_ctors(def, cs)),
        }
    }
}

/// Returns the fields a rekey of `rel` carries: each field of a user type whose value holds a
/// value of `Key rel`, but a field of `rel`'s own type typed by its key, as its type, its
/// constructor when its type has more than the one of its name, and its name.
pub(super) fn carried_fields(schema: &Schema, rel: RelId) -> Vec<(TypeId, Option<String>, String)> {
    let own = schema.relation(rel).elem_def;
    let mut needed: Vec<(TypeId, Option<String>, String)> = Vec::new();
    for (j, t) in schema.types.iter().enumerate() {
        let def = TypeId(j as u32);
        if schema.module(t.module).engine {
            continue;
        }
        let carried = |ty: &Ty| refers(schema, ty, rel) && !(def == own && keys(ty, rel));
        for f in t.header() {
            if carried(&f.ty) {
                needed.push((def, None, f.name.clone()));
            }
        }
        for c in t.ctors() {
            for f in &c.fields {
                if carried(&f.ty) {
                    let ctor = (t.ctors().len() > 1 || c.name != t.name).then(|| c.name.clone());
                    needed.push((def, ctor, f.name.clone()));
                }
            }
        }
    }
    needed
}

/// Returns whether a value of `t` holds a value of `Key rel`, directly or through the key of a
/// relation it references.
pub(super) fn refers(schema: &Schema, t: &Ty, rel: RelId) -> bool {
    let mut rels = BTreeSet::new();
    schema.reached(t, &mut BTreeSet::new(), &mut rels);
    rels.contains(&rel)
}

/// Returns whether `t` is `Key rel` or a declared type with it among its arguments.
fn keys(t: &Ty, rel: RelId) -> bool {
    match t {
        Ty::Key { rel: r, .. } => *r == RelRef::Declared(rel),
        Ty::Data { args, .. } => args.iter().any(|a| keys(a, rel)),
        _ => false,
    }
}

pub(super) fn path_text(p: &ast::FieldPath) -> String {
    match &p.ctor {
        Some(c) => format!("{}.{}.{}", p.ty.name.text, c.text, p.field.text),
        None => format!("{}.{}", p.ty.name.text, p.field.text),
    }
}
