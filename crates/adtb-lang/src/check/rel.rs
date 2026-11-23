use super::*;
use crate::kernel::Kind;
use crate::textual::key_names;

impl Checker<'_> {
    /// `r.S` as a set of rows.
    pub(super) fn rel_out(&mut self, rel: RelId, ctors: Option<CtorSet>) -> Out {
        let r = self.schema.relation(rel);
        let ctors = ctors.and_then(|s| self.schema.normalize_ctors(r.elem_def, s));
        let elem = match &r.elem {
            Ty::Data { def, args, .. } => Ty::Data {
                def: *def,
                args: args.clone(),
                ctors: ctors.clone(),
            },
            t => t.clone(),
        };
        let tm = self.b().rel(rel, ctors);
        Out {
            tm,
            ty: Ty::Set(Box::new(elem)),
            rows: Some(rel),
            provenance: Provenance::Snapshot,
        }
    }

    pub(super) fn elem_ty(&self, ty: &Ty) -> Option<Ty> {
        ty::elem(ty).cloned()
    }

    pub(super) fn fields_of(&self, ty: &Ty) -> Vec<(String, Ty)> {
        ty::fields(self.schema, ty)
    }

    /// The relation whose single key field is a `Key` of itself declared in `def`, with that field.
    pub(super) fn self_key(&self, def: TypeId) -> Option<(RelId, String)> {
        self.schema.relations.iter().enumerate().find_map(|(i, r)| {
            let rel = RelId(i as u32);
            match r.key.as_slice() {
                [k] if r.elem_def == def && self.schema.self_key(rel, k) => Some((rel, k.clone())),
                _ => None,
            }
        })
    }

    /// The type of the value `r.key(v)` takes and `value(k)` returns.
    pub(super) fn key_value_ty(&self, rel: RelId) -> Result<Ty, String> {
        key_value_ty(self.schema, rel)
    }

    /// The constructor set of a row or key type, `None` for every constructor.
    pub(super) fn ctors_of(ty: &Ty) -> Option<CtorSet> {
        match ty {
            Ty::Data { ctors, .. } => ctors.clone(),
            Ty::Key { ctors, .. } => match ctors {
                KeyCtors::All | KeyCtors::Late(_) => None,
                KeyCtors::Only(cs) => Some(cs.clone()),
            },
            _ => None,
        }
    }

    /// Returns whether ordering by `keys`, each over `x`, gives distinct values to distinct elements.
    pub(super) fn determinate(&self, x: Var, elem: &Ty, rows: Option<RelId>, keys: &[Tm]) -> bool {
        let is_x = |t: Tm| matches!(self.terms.kind(t), Kind::Var(v) if *v == x);
        if matches!(elem, Ty::Key { .. }) {
            return keys.iter().any(|&k| is_x(k));
        }
        let Some(rel) = rows else {
            return false;
        };
        if keys
            .iter()
            .any(|&k| matches!(self.terms.kind(k), Kind::Key(inner) if is_x(*inner)))
        {
            return true;
        }
        let fields: Vec<&str> = keys
            .iter()
            .filter_map(|&k| match self.terms.kind(k) {
                Kind::Field(inner, f) if is_x(*inner) => Some(f.as_str()),
                _ => None,
            })
            .collect();
        let r = self.schema.relation(rel);
        if r.key.iter().all(|k| fields.contains(&k.as_str())) {
            return true;
        }
        let ctors = Self::ctors_of(elem);
        r.indexes.iter().any(|ix| {
            ix.unique
                && match (&ix.scope, &ctors) {
                    (None, _) => true,
                    (Some(_), None) => false,
                    (Some(s), Some(c)) => c.iter().all(|i| s.contains(i)),
                }
                && ix.fields.iter().all(|e| match &e.kind {
                    ast::ExprKind::Name(q) => fields.contains(&q.name.text.as_str()),
                    _ => false,
                })
        })
    }
}

/// Returns the type of the value `r.key(v)` takes and `value(k)` returns.
pub(crate) fn key_value_ty(schema: &Schema, rel: RelId) -> Result<Ty, String> {
    let r = schema.relation(rel);
    let def = schema.ty(r.elem_def);
    let field_ty = |name: &str| -> Result<Ty, String> {
        let f = def
            .field_named(0, name)
            .ok_or_else(|| format!("`{}` has no key field `{name}`", r.name))?;
        match &f.ty {
            _ if schema.self_key(rel, name) => schema.generated_key_ty(rel).ok_or_else(|| {
                format!(
                    "`{}` identifies its rows by `Key {}` with no generator, so its key has no value",
                    r.name, r.name
                )
            }),
            t => Ok(t.subst(r.elem_args())),
        }
    };
    match r.key.as_slice() {
        [one] => field_ty(one),
        _ => {
            let mut fs = Vec::new();
            for k in key_names(schema, rel) {
                let ty = field_ty(&k)?;
                fs.push((k, ty));
            }
            Ok(Ty::Record(Fields::new(fs)))
        }
    }
}
