use super::value::{CopyToken, Keyed, Shape};
use super::*;
use crate::ids::Slot;
use crate::kernel::Transition;
use crate::schema::TypeDefKind;

impl Analyzer<'_> {
    pub(super) fn unknown(&self, ty: &Ty) -> Value {
        match ty {
            Ty::Base(_) | Ty::Key { .. } | Ty::Param(_) | Ty::Fn(..) => Value::scalar(),
            Ty::Set(t) | Ty::NonEmpty(t) => Value::set(self.unknown(t)),
            Ty::Unit => Value::row(None, BTreeMap::new()),
            Ty::Change => Value::change(vec![Vec::new()]),
            Ty::Record(fields) => Value::row(
                None,
                fields
                    .iter()
                    .map(|(n, t)| (n.clone(), self.unknown(t)))
                    .collect(),
            ),
            Ty::Data { def, args, ctors } => match &self.schema.ty(*def).kind {
                TypeDefKind::Newtype { base, .. } => self.unknown(&base.subst(args)),
                TypeDefKind::Sum { .. } => {
                    let mut out = Value::default();
                    for c in self.constructors(*def, ctors.as_deref()) {
                        let fields = self
                            .fields(*def, args, c)
                            .into_iter()
                            .map(|(n, t, _)| (n, self.unknown(&t)))
                            .collect();
                        out.join(Value::row(Some(c), fields));
                    }
                    out
                }
            },
            Ty::Sum { ctors, sel } => {
                let mut out = Value::default();
                for (c, (_, fields)) in ctors.iter().enumerate() {
                    if sel.as_ref().is_none_or(|s| s.contains(&(c as u32))) {
                        out.join(Value::row(
                            Some(c as u32),
                            fields
                                .iter()
                                .map(|(n, t)| (n.clone(), self.unknown(t)))
                                .collect(),
                        ));
                    }
                }
                out
            }
            Ty::Tag(t) => {
                let mut out = self.unknown(t);
                if let Shape::Rows(rows) = &mut out.shape {
                    for fields in rows.values_mut() {
                        fields.clear();
                    }
                }
                out
            }
            Ty::Group(a, b) => Value::row(
                None,
                BTreeMap::from([
                    ("key".to_string(), self.unknown(a)),
                    ("rows".to_string(), Value::set(self.unknown(b))),
                ]),
            ),
        }
    }

    pub(super) fn constructors(&self, def: TypeId, selected: Option<&[u32]>) -> Vec<u32> {
        selected
            .map(<[u32]>::to_vec)
            .unwrap_or_else(|| (0..self.schema.ty(def).ctors().len() as u32).collect())
    }

    fn fields(&self, def: TypeId, args: &[Ty], ctor: u32) -> Vec<(String, Ty, bool)> {
        let ty = self.schema.ty(def);
        ty.header()
            .iter()
            .map(|f| (f.name.clone(), f.ty.subst(args), true))
            .chain(
                ty.ctors()[ctor as usize]
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), f.ty.subst(args), false)),
            )
            .collect()
    }

    pub(super) fn target(&self, rel: RelId, ctor: u32) -> Result<Target, Error> {
        let def = self.schema.relation(rel).elem_def;
        Ok(Target {
            relation: RelationId(
                *self
                    .schema
                    .ids
                    .relations
                    .get(&rel)
                    .ok_or(Error::MissingIdentity(Slot::Relation(rel)))?,
            ),
            constructor: ConstructorId(
                *self
                    .schema
                    .ids
                    .ctors
                    .get(&(def, ctor))
                    .ok_or(Error::MissingIdentity(Slot::Ctor(def, ctor)))?,
            ),
        })
    }

    pub(super) fn field_id(&self, rel: RelId, ctor: u32, name: &str) -> Result<FieldId, Error> {
        let def = self.schema.relation(rel).elem_def;
        let own = (!self.schema.ty(def).header().iter().any(|f| f.name == name)).then_some(ctor);
        let key = (def, own, name.to_string());
        self.schema
            .ids
            .fields
            .get(&key)
            .copied()
            .map(FieldId)
            .ok_or_else(|| Error::MissingIdentity(Slot::Field(def, own, name.to_string())))
    }

    pub(super) fn relation(
        &mut self,
        rel: RelId,
        selected: Option<&[u32]>,
        transition: Option<Transition>,
        range: bool,
    ) -> Result<Value, Error> {
        let r = self.schema.relation(rel);
        let def = r.elem_def;
        let args = match &r.elem {
            Ty::Data { args, .. } => args.clone(),
            _ => return Err(Error::InvalidKernel("relation has a declared row type")),
        };
        let keys = r.key.clone();
        let mut out = Value::default();
        for c in self.constructors(def, selected) {
            let target = self.target(rel, c)?;
            for key in &keys {
                let field = self.field_id(rel, c, key)?;
                self.requirements.keys.insert((target, field));
            }
            if range {
                self.range(target);
            }
            let mut fields = BTreeMap::new();
            for (name, ty, _) in self.fields(def, &args, c) {
                let field = self.field_id(rel, c, &name)?;
                let mut value = self.unknown(&ty);
                value.reads.insert(Requirement::ReadField(target, field));
                value.copy = Some(CopyToken(self.fresh()));
                fields.insert(name, value);
            }
            let mut row = Value::row(Some(c), fields);
            row.copy = Some(CopyToken(self.fresh()));
            row.stored.insert(rel);
            row.keyed = Some(Keyed {
                rel,
                snapshot: self.snapshot,
                transition,
            });
            out.join(row);
        }
        Ok(Value::set(out))
    }

    pub(super) fn range(&mut self, target: Target) {
        self.requirements
            .required
            .insert(Requirement::ReadConstructor(target));
        for &(t, f) in &self.requirements.keys {
            if t == target {
                self.requirements
                    .required
                    .insert(Requirement::ReadField(t, f));
            }
        }
    }
}
