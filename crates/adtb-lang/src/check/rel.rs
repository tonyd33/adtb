use super::*;
use crate::kernel::Kind;

impl Checker<'_> {
    /// `r.S` as a set of rows.
    pub(super) fn rel_out(&mut self, rel: RelId, ctors: Option<CtorSet>) -> Out {
        let r = self.schema.relation(rel);
        let elem = match &r.elem {
            Ty::Data { def, args, .. } => Ty::Data {
                def: *def,
                args: args.clone(),
                ctors: ctors.clone(),
            },
            t => t.clone(),
        };
        let tm = self.add(Kind::Rel(rel, ctors), Ty::Set(Box::new(elem.clone())));
        Out {
            tm,
            ty: Ty::Set(Box::new(elem)),
            rows: Some(rel),
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
            if r.elem_def != def || r.key.len() != 1 {
                return None;
            }
            let f = self.schema.ty(def).field(0, &r.key[0])?;
            matches!(f.ty, Ty::Key { rel: own, .. } if own == rel).then(|| (rel, r.key[0].clone()))
        })
    }

    /// The type of the value `r.key(v)` takes and `value(k)` returns.
    pub(super) fn key_value_ty(&self, rel: RelId) -> Result<Ty, String> {
        let r = self.schema.relation(rel);
        let def = self.schema.ty(r.elem_def);
        let field_ty = |name: &str| -> Result<Ty, String> {
            let f = def
                .field(0, name)
                .ok_or_else(|| format!("`{}` has no key field `{name}`", r.name))?;
            match &f.ty {
                Ty::Key { rel: own, .. } if *own == rel => match r.generate {
                    Some(crate::schema::Generator::Sequence) => Ok(Ty::Base(Base::I64)),
                    Some(crate::schema::Generator::Uuidv7) => Ok(Ty::Data {
                        def: self.schema.prelude_type("Uuid"),
                        args: Vec::new(),
                        ctors: None,
                    }),
                    _ => Err(format!(
                        "`{}` identifies its rows by `Key {}` with no generator, so its key has no value",
                        r.name, r.name
                    )),
                },
                t => Ok(t.subst(r.elem_args())),
            }
        };
        match r.key.as_slice() {
            [one] => field_ty(one),
            many => {
                let mut fs = Vec::new();
                for k in many {
                    fs.push((k.clone(), field_ty(k)?));
                }
                fs.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(Ty::Record(fs))
            }
        }
    }

    /// The constructor set of a row or key type, `None` for every constructor.
    pub(super) fn ctors_of(ty: &Ty) -> Option<CtorSet> {
        match ty {
            Ty::Data { ctors, .. } | Ty::Key { ctors, .. } => ctors.clone(),
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
