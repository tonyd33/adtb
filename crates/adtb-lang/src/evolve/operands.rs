use super::*;
use crate::ids::{KeyKind, Owner, Space};

/// Ties each operand of an operator that names an object to the object's catalog id, resolved by
/// key in the source before the operator, or, for the field an `add field` or `add computed`
/// declares, in the source after it. The name a rename gives and every other name an operator
/// declares stay untied.
struct Operands<'a> {
    before: &'a Source,
    after: &'a Source,
    module: &'a str,
    out: &'a mut Names,
}

/// Records in `out` the catalog id of each object operator `op` of module `module` names by an
/// operand, `before` and `after` being the sources around it.
pub(super) fn tie_operands(
    before: &Source,
    after: &Source,
    module: &str,
    op: &Operator,
    out: &mut Names,
) {
    let mut o = Operands {
        before,
        after,
        module,
        out,
    };
    o.operator(&op.kind);
}

fn key_in(src: &Source, kind: KeyKind, owner: Owner, name: &str) -> Option<u64> {
    src.ids.get(&ObjectKey::named(kind, owner, name)).copied()
}

impl Operands<'_> {
    fn module_of(&self, q: &QName) -> String {
        q.module
            .as_ref()
            .map_or_else(|| self.module.to_string(), |m| m.text.clone())
    }

    fn tie(&mut self, span: Span, space: Space, id: Option<u64>) -> Option<u64> {
        if let Some(id) = id {
            self.out.tie(span, (space, id));
        }
        id
    }

    fn ty(&mut self, q: &QName) -> Option<u64> {
        let id = key_in(
            self.before,
            KeyKind::Type,
            Owner::Module(self.module_of(q)),
            &q.name.text,
        );
        self.tie(q.name.span, Space::Shared, id)
    }

    fn relation(&mut self, q: &QName) -> Option<u64> {
        let id = key_in(
            self.before,
            KeyKind::Relation,
            Owner::Module(self.module_of(q)),
            &q.name.text,
        );
        self.tie(q.name.span, Space::Shared, id)
    }

    /// Returns the id of the type relation `q` is declared over.
    fn relation_type(&self, q: &QName) -> Option<u64> {
        let (m, t) = self
            .before
            .relation_type(&self.module_of(q), &q.name.text)?;
        key_in(self.before, KeyKind::Type, Owner::Module(m), &t)
    }

    fn ctor(&mut self, ty: Option<u64>, c: &ast::Ident) -> Option<u64> {
        let id = key_in(
            self.before,
            KeyKind::Ctor,
            Owner::Object(Space::Shared, ty?),
            &c.text,
        );
        self.tie(c.span, Space::Shared, id)
    }

    /// Ties field `f` of constructor `ctor` of type `ty`, else of its header, else of the
    /// constructor a record type `t` names after itself, in the source after the operator when
    /// `declared`.
    fn field(
        &mut self,
        ty: Option<u64>,
        t: &str,
        ctor: Option<u64>,
        f: &ast::Ident,
        declared: bool,
    ) {
        let Some(ty) = ty else { return };
        let src = if declared { self.after } else { self.before };
        let at = |owner: u64| {
            key_in(
                src,
                KeyKind::Field,
                Owner::Object(Space::Shared, owner),
                &f.text,
            )
        };
        let id = match ctor {
            Some(c) => at(c),
            None => at(ty).or_else(|| {
                let record = key_in(src, KeyKind::Ctor, Owner::Object(Space::Shared, ty), t)?;
                at(record)
            }),
        };
        self.tie(f.span, Space::Field, id);
    }

    fn path(&mut self, p: &ast::FieldPath, declared: bool) {
        let ty = self.ty(&p.ty);
        let ctor = p.ctor.as_ref().and_then(|c| self.ctor(ty, c));
        self.field(ty, &p.ty.name.text, ctor, &p.field, declared);
    }

    fn ctor_path(&mut self, p: &ast::CtorPath) -> Option<u64> {
        let ty = self.ty(&p.ty);
        self.ctor(ty, &p.ctor);
        ty
    }

    /// Ties each of `names`, a constructor of type `ty` or a named set of relation `rel`.
    fn sel(&mut self, rel: Option<u64>, ty: Option<u64>, names: &[&ast::Ident]) {
        for n in names {
            if self.ctor(ty, n).is_none()
                && let Some(rel) = rel
            {
                let id = key_in(
                    self.before,
                    KeyKind::Set,
                    Owner::Object(Space::Shared, rel),
                    &n.text,
                );
                self.tie(n.span, Space::Set, id);
            }
        }
    }

    fn relation_ref(&mut self, r: &ast::RelationRef) -> (Option<u64>, Option<u64>) {
        let rel = self.relation(&r.relation);
        let ty = self.relation_type(&r.relation);
        if let Some(sel) = &r.sel {
            self.sel(rel, ty, &sel_idents(sel));
        }
        (rel, ty)
    }

    fn function(&mut self, q: &QName) -> Option<u64> {
        let id = key_in(
            self.before,
            KeyKind::Function,
            Owner::Module(self.module_of(q)),
            &q.name.text,
        );
        self.tie(q.name.span, Space::Function, id)
    }

    fn constraint(&mut self, q: &QName) -> Option<u64> {
        let id = key_in(
            self.before,
            KeyKind::Constraint,
            Owner::Module(self.module_of(q)),
            &q.name.text,
        );
        self.tie(q.name.span, Space::Constraint, id)
    }

    fn arm_targets<'x>(&mut self, arms: impl IntoIterator<Item = &'x ast::ArmClause>) {
        for a in arms {
            if let Some(f) = &a.field {
                let rel = self.relation(&a.target);
                self.computed(rel, f, false);
            } else if self.function(&a.target).is_none() {
                self.constraint(&a.target);
            }
        }
    }

    /// Ties computed field `f` of relation `rel`, in the source after the operator when
    /// `declared`.
    fn computed(&mut self, rel: Option<u64>, f: &ast::Ident, declared: bool) {
        let Some(rel) = rel else { return };
        let id = key_in(
            if declared { self.after } else { self.before },
            KeyKind::Computed,
            Owner::Object(Space::Shared, rel),
            &f.text,
        );
        self.tie(f.span, Space::Computed, id);
    }

    fn target(&mut self, t: &CtorTarget) {
        match t {
            CtorTarget::Relation(r) => {
                self.relation_ref(r);
            }
            CtorTarget::Type(p) => {
                self.ctor_path(p);
            }
        }
    }

    /// Ties field `f` of the row of relation `q`.
    fn row_field(&mut self, q: &QName, ctor: Option<&ast::Ident>, f: &ast::Ident, declared: bool) {
        let ty = self.relation_type(q);
        let t = self
            .before
            .relation_type(&self.module_of(q), &q.name.text)
            .map(|(_, t)| t)
            .unwrap_or_default();
        let ctor = ctor.and_then(|c| self.ctor(ty, c));
        self.field(ty, &t, ctor, f, declared);
    }

    fn operator(&mut self, kind: &OperatorKind) {
        match kind {
            OperatorKind::DropType(t) | OperatorKind::RenameType(t, _) => {
                self.ty(t);
            }
            OperatorKind::DropRelation(r) | OperatorKind::RenameRelation(r, _) => {
                self.relation(r);
            }
            OperatorKind::DropSet(set, r) | OperatorKind::RenameSet(set, r, _) => {
                let rel = self.relation(r);
                self.sel(rel, None, &[set]);
            }
            OperatorKind::AddConstructor {
                path,
                place,
                joins,
                arms,
                ..
            } => {
                let ty = self.ty(&path.ty);
                if let Some(CtorPlace::Before(c) | CtorPlace::After(c)) = place {
                    self.ctor(ty, c);
                }
                for j in joins {
                    self.relation_ref(j);
                }
                self.arm_targets(arms);
            }
            OperatorKind::RemoveConstructor(p) | OperatorKind::RenameConstructor(p, _) => {
                self.ctor_path(p);
            }
            OperatorKind::SplitConstructor { path, clauses, .. } => {
                self.ctor_path(path);
                self.arm_targets(clauses.iter().filter_map(|c| match c {
                    ast::SplitClause::Arm(a) => Some(&**a),
                    _ => None,
                }));
            }
            OperatorKind::MergeConstructor {
                ty, sel, clauses, ..
            }
            | OperatorKind::NestConstructor {
                ty, sel, clauses, ..
            } => {
                let t = self.ty(ty);
                self.sel(None, t, &sel_idents(sel));
                self.arm_targets(clauses);
            }
            OperatorKind::UnnestConstructor(p, clauses) => {
                self.ctor_path(p);
                self.arm_targets(clauses);
            }
            OperatorKind::AddField { path, .. } => self.path(path, true),
            OperatorKind::AlterField { path, .. }
            | OperatorKind::DropField(path)
            | OperatorKind::RenameField(path, _) => self.path(path, false),
            OperatorKind::MoveField { path, to, sel, .. } => {
                self.path(path, false);
                let ty = self.ty(to);
                if let Some(sel) = sel {
                    self.sel(None, ty, &sel_idents(sel));
                }
            }
            OperatorKind::AddUnique(_, on)
            | OperatorKind::DropUnique(_, on)
            | OperatorKind::AddIndex { on, .. }
            | OperatorKind::DropIndex { on, .. } => {
                self.relation_ref(on);
            }
            OperatorKind::Rekey {
                relation,
                key,
                carry,
            } => {
                self.relation(relation);
                for k in key {
                    self.row_field(relation, None, k, false);
                }
                for p in carry {
                    self.path(p, false);
                }
            }
            OperatorKind::AddDefault {
                relation,
                ctor,
                field,
                ..
            }
            | OperatorKind::DropDefault {
                relation,
                ctor,
                field,
            } => {
                self.relation(relation);
                self.row_field(relation, ctor.as_ref(), field, false);
            }
            OperatorKind::RedefineFn(f) => {
                self.function(&QName {
                    module: None,
                    name: f.name.clone(),
                });
            }
            OperatorKind::DropFn(f) | OperatorKind::DropAggregate(f) | OperatorKind::Unstore(f) => {
                self.function(f);
            }
            OperatorKind::DropConstraint(c) | OperatorKind::DropCheck(c) => {
                self.constraint(c);
            }
            OperatorKind::AddComputed {
                relation, field, ..
            } => {
                self.relation(relation);
                let owner = Owner::Module(self.module_of(relation));
                let rel = key_in(self.after, KeyKind::Relation, owner, &relation.name.text);
                self.computed(rel, field, true);
            }
            OperatorKind::RedefineComputed {
                relation, field, ..
            }
            | OperatorKind::DropComputed(relation, field) => {
                let rel = self.relation(relation);
                self.computed(rel, field, false);
            }
            OperatorKind::Enable(t) | OperatorKind::Disable(t) | OperatorKind::Deprecate(t) => {
                self.target(t);
            }
            OperatorKind::Revert { fills, .. } => {
                for (p, _) in fills {
                    self.path(p, false);
                }
            }
            OperatorKind::AddModule(_)
            | OperatorKind::DropModule(_)
            | OperatorKind::AddType(_)
            | OperatorKind::AddRelation(_)
            | OperatorKind::AddSet(_)
            | OperatorKind::DefineFn(_)
            | OperatorKind::AddConstraint(_)
            | OperatorKind::AddCheck(_)
            | OperatorKind::Store(_)
            | OperatorKind::DefineAggregate(_)
            | OperatorKind::Script(_)
            | OperatorKind::Install { .. }
            | OperatorKind::Uninstall(_)
            | OperatorKind::AdoptTzdb { .. }
            | OperatorKind::Grant(_)
            | OperatorKind::Revoke(_) => {}
        }
    }
}
