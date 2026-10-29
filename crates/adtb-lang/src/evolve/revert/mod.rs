use super::*;

mod inverse;

/// How an inverse applies an operator of its text.
#[derive(Clone, Copy)]
pub(super) enum How<'a> {
    /// As the operator written.
    Written,
    /// A merge whose arms' fixed values tell its constructors apart.
    Derived,
    /// A move to the header of the field of every constructor of the slice.
    UpFrom(&'a [String]),
}

/// One step of the inverse of an operator.
#[derive(Clone, Debug)]
pub(super) enum Inverse {
    /// Operators, as source text, applied in the migration's module.
    Ops(Written),
    /// A merge, as source text, putting back a split: its arms' fixed values tell its
    /// constructors apart.
    Derived(Written),
    /// A split, as source text, putting back a merge, and the texts of the checks it carried, by
    /// module and name, set in the same step.
    Unmerge {
        split: Written,
        checks: Vec<(String, String, String)>,
    },
    /// A move to the header, as source text naming the first of `from`, of the field of every
    /// constructor of `from`.
    MoveUp { text: Written, from: Vec<String> },
    /// Remove the constructor an `add constructor` added, with its arms, then set the type back
    /// to `record`, the text of the record it was.
    Unadd {
        path: ast::CtorPath,
        record: Option<String>,
    },
    /// Puts back a removed constructor, as written, after the constructor named, else first.
    Readd {
        ty: QName,
        text: String,
        after: Option<String>,
        arms: Vec<(String, Vec<String>)>,
    },
    /// Sets write states back, `None` removing the state.
    States(Vec<(bool, StateKey, Option<State>)>),
    /// Set what each role holds of a privilege back, each privilege naming objects by the ids
    /// `ids` gives their paths.
    Grants {
        back: Vec<HeldGrant>,
        ids: BTreeMap<String, u64>,
    },
    /// Sets the declaration of a kind and name to a text, `None` removing it.
    Decl {
        module: String,
        kind: &'static str,
        name: String,
        text: Option<String>,
        states: Vec<(StateKey, State)>,
    },
    /// Add module `name` with the declaration texts `decls`, or with `present` false remove it.
    Module {
        name: String,
        present: bool,
        installed: bool,
        decls: Vec<String>,
    },
    /// Applies a logged migration's operators again.
    Replay(Box<Logged>),
    /// Sets a relation's key clause to its text, generator included.
    KeyClause { relation: QName, text: String },
    /// Puts the fields of a constructor, or of the header when `ctor` is `None`, in `order`.
    Reorder {
        ty: QName,
        ctor: Option<String>,
        order: Vec<String>,
    },
}

/// Returns the step that sets back each privilege an operator changed for a role, as `before`
/// held it, from `after`; `None` when it changed none.
fn grants_back(before: &Source, after: &Source) -> Option<Inverse> {
    let held = |grants: &[LogGrant], g: &LogGrant| {
        grants
            .iter()
            .position(|x| x.role == g.role && x.privilege == g.privilege)
            .map(|i| (i, grants[i].grant_option))
    };
    let mut back = Vec::new();
    for g in before.grants.iter().chain(&after.grants) {
        let was = held(&before.grants, g);
        let is = held(&after.grants, g);
        let changed = was.map(|(_, o)| o) != is.map(|(_, o)| o);
        if changed
            && !back
                .iter()
                .any(|h: &HeldGrant| h.role == g.role && h.privilege == g.privilege)
        {
            back.push(HeldGrant {
                role: g.role.clone(),
                privilege: g.privilege.clone(),
                held: was,
            });
        }
    }
    (!back.is_empty()).then(|| Inverse::Grants {
        back,
        ids: before.ids.clone(),
    })
}

/// The objects an operator creates, by kind and name, the module first.
pub(super) fn created(op: &OperatorKind, module: &str) -> Vec<String> {
    let q = |n: &QName| {
        n.module
            .as_ref()
            .map_or(module.to_string(), |m| m.text.clone())
    };
    match op {
        OperatorKind::AddModule(m) | OperatorKind::Install { name: m, .. } => {
            vec![format!("module {}", m.text)]
        }
        OperatorKind::AddType(t) => {
            let mut out = vec![format!("type {module}::{}", t.name.text)];
            if let ast::TypeBody::Ctors(cs) = &t.body {
                out.extend(
                    cs.iter()
                        .map(|c| format!("ctor {module}::{}.{}", t.name.text, c.name.text)),
                );
            }
            out
        }
        OperatorKind::AddRelation(r) => vec![format!("relation {module}::{}", r.name.name.text)],
        OperatorKind::AddSet(s) => {
            vec![format!("set {module}::{}.{}", s.relation.text, s.name.text)]
        }
        OperatorKind::AddConstructor { path, .. } => {
            vec![format!(
                "ctor {}::{}.{}",
                q(&path.ty),
                path.ty.name.text,
                path.ctor.text
            )]
        }
        OperatorKind::SplitConstructor { path, into, .. } => into
            .iter()
            .map(|d| {
                format!(
                    "ctor {}::{}.{}",
                    q(&path.ty),
                    path.ty.name.text,
                    d.name.text
                )
            })
            .collect(),
        OperatorKind::MergeConstructor { ty, into, .. }
        | OperatorKind::NestConstructor { ty, into, .. } => {
            vec![format!(
                "ctor {}::{}.{}",
                q(ty),
                ty.name.text,
                into.name.text
            )]
        }
        OperatorKind::AddField { path, .. } => vec![format!(
            "field {}::{}.{}.{}",
            q(&path.ty),
            path.ty.name.text,
            path.ctor.as_ref().map_or("", |c| c.text.as_str()),
            path.field.text
        )],
        OperatorKind::DefineFn(f) => vec![format!("fn {module}::{}", f.name.text)],
        OperatorKind::DefineAggregate(a) => vec![format!("fn {module}::{}", a.name.text)],
        OperatorKind::AddConstraint(c) => vec![format!("constraint {module}::{}", c.name.text)],
        OperatorKind::AddCheck(c) => vec![format!("constraint {module}::{}", c.name.text)],
        OperatorKind::Store(s) => vec![format!("view {module}::{}", s.view.name.text)],
        OperatorKind::AddComputed {
            relation, field, ..
        } => {
            vec![format!(
                "computed {}::{}.{}",
                q(relation),
                relation.name.text,
                field.text
            )]
        }
        _ => Vec::new(),
    }
}

/// The objects an operator names, by kind and name as [`created`] gives them, read against `schema`.
pub(super) fn named(op: &OperatorKind, module: &str, schema: &Schema) -> Vec<String> {
    let q = |n: &QName| {
        n.module
            .as_ref()
            .map_or(module.to_string(), |m| m.text.clone())
    };
    let ty_of = |r: &QName| -> Option<String> {
        let mid = schema.module_named(&q(r))?;
        match schema.module(mid).values.get(&r.name.text)? {
            ValueDef::Relation(rel) => Some(schema.ty(schema.relation(*rel).elem_def).name.clone()),
            _ => None,
        }
    };
    let ctors_of = |r: &ast::RelationRef| -> Vec<String> {
        let Some(t) = ty_of(&r.relation) else {
            return Vec::new();
        };
        let mut out = vec![format!(
            "relation {}::{}",
            q(&r.relation),
            r.relation.name.text
        )];
        if let Some(sel) = &r.sel {
            for x in sel_idents(sel) {
                out.push(format!("ctor {}::{t}.{}", q(&r.relation), x.text));
                out.push(format!(
                    "set {}::{}.{}",
                    q(&r.relation),
                    r.relation.name.text,
                    x.text
                ));
            }
        }
        out
    };
    let ctor = |t: &QName, c: &ast::Ident| format!("ctor {}::{}.{}", q(t), t.name.text, c.text);
    let field = |p: &ast::FieldPath| {
        format!(
            "field {}::{}.{}.{}",
            q(&p.ty),
            p.ty.name.text,
            p.ctor.as_ref().map_or("", |c| c.text.as_str()),
            p.field.text
        )
    };
    let mut out = Vec::new();
    match op {
        OperatorKind::Enable(t) | OperatorKind::Disable(t) | OperatorKind::Deprecate(t) => {
            match t {
                CtorTarget::Relation(r) => out.extend(ctors_of(r)),
                CtorTarget::Type(p) => out.push(ctor(&p.ty, &p.ctor)),
            }
        }
        OperatorKind::RemoveConstructor(p)
        | OperatorKind::RenameConstructor(p, _)
        | OperatorKind::UnnestConstructor(p, _) => {
            out.push(ctor(&p.ty, &p.ctor));
        }
        OperatorKind::SplitConstructor { path, .. } => out.push(ctor(&path.ty, &path.ctor)),
        OperatorKind::MergeConstructor { ty, sel, .. }
        | OperatorKind::NestConstructor { ty, sel, .. } => {
            out.extend(sel_idents(sel).into_iter().map(|c| ctor(ty, c)));
        }
        OperatorKind::AddConstructor {
            path, joins, arms, ..
        } => {
            out.push(format!("type {}::{}", q(&path.ty), path.ty.name.text));
            for j in joins {
                out.extend(ctors_of(j));
            }
            for a in arms {
                out.push(format!("fn {}::{}", q(&a.target), a.target.name.text));
                out.push(format!(
                    "constraint {}::{}",
                    q(&a.target),
                    a.target.name.text
                ));
            }
        }
        OperatorKind::DropType(t) | OperatorKind::RenameType(t, _) => {
            out.push(format!("type {}::{}", q(t), t.name.text));
        }
        OperatorKind::DropRelation(r) | OperatorKind::RenameRelation(r, _) => {
            out.push(format!("relation {}::{}", q(r), r.name.text));
        }
        OperatorKind::DropSet(s, r) | OperatorKind::RenameSet(s, r, _) => {
            out.push(format!("set {}::{}.{}", q(r), r.name.text, s.text));
        }
        OperatorKind::AddField { path, .. } => {
            out.push(format!("type {}::{}", q(&path.ty), path.ty.name.text));
            if let Some(c) = &path.ctor {
                out.push(ctor(&path.ty, c));
            }
        }
        OperatorKind::DropField(p)
        | OperatorKind::AlterField { path: p, .. }
        | OperatorKind::RenameField(p, _)
        | OperatorKind::MoveField { path: p, .. } => {
            out.push(field(p));
        }
        OperatorKind::AddUnique(_, on)
        | OperatorKind::DropUnique(_, on)
        | OperatorKind::AddIndex { on, .. }
        | OperatorKind::DropIndex { on, .. } => out.extend(ctors_of(on)),
        OperatorKind::Rekey {
            relation, carry, ..
        } => {
            out.push(format!("relation {}::{}", q(relation), relation.name.text));
            out.extend(carry.iter().map(field));
        }
        OperatorKind::AddDefault { relation, .. } | OperatorKind::DropDefault { relation, .. } => {
            out.push(format!("relation {}::{}", q(relation), relation.name.text));
        }
        OperatorKind::RedefineFn(f) => out.push(format!("fn {module}::{}", f.name.text)),
        OperatorKind::DropFn(f) | OperatorKind::DropAggregate(f) => {
            out.push(format!("fn {}::{}", q(f), f.name.text));
        }
        OperatorKind::DropConstraint(c) | OperatorKind::DropCheck(c) => {
            out.push(format!("constraint {}::{}", q(c), c.name.text));
        }
        OperatorKind::Unstore(v) => out.push(format!("view {}::{}", q(v), v.name.text)),
        OperatorKind::RedefineComputed {
            relation, field, ..
        }
        | OperatorKind::DropComputed(relation, field) => {
            out.push(format!(
                "computed {}::{}.{}",
                q(relation),
                relation.name.text,
                field.text
            ));
        }
        OperatorKind::DropModule(m) | OperatorKind::Uninstall(m) => {
            out.push(format!("module {}", m.text));
        }
        _ => {}
    }
    out
}

impl Cx<'_> {
    pub(super) fn revert(&mut self, migration: &str, fills: &[(ast::FieldPath, Fill)]) -> R<()> {
        let number: u64 = migration.parse().unwrap_or(u64::MAX);
        let Some(pos) = self.src.log.iter().position(|l| l.number == number) else {
            return self.err(format!("migration {migration} is not in the log"));
        };
        let logged = self.src.log[pos].clone();
        if logged
            .operators
            .iter()
            .any(|o| matches!(o.kind, OperatorKind::Script(_)))
        {
            return self.err(format!(
                "the inverse of migration {migration} is undefined: it holds a `script`"
            ));
        }
        let made: BTreeSet<String> = logged
            .operators
            .iter()
            .flat_map(|o| created(&o.kind, &logged.module))
            .collect();
        for later in &self.src.log[pos + 1..] {
            let Ok(schema) = later.before.schema() else {
                continue;
            };
            let depends = later.operators.iter().any(|o| {
                named(&o.kind, &later.module, &schema)
                    .iter()
                    .any(|n| made.contains(n))
            });
            if depends {
                return self.err(format!(
                    "`revert {migration}` is refused while migration {:04} depends on it",
                    later.number
                ));
            }
        }
        let mut state = (*logged.before).clone();
        state.version = logged.number;
        state.expecting(&logged.module, &logged.operators);
        let mut steps = Vec::new();
        for op in &logged.operators {
            let mut step = self.inverse(&state, &logged, op, fills)?;
            let before = state.clone();
            if let Err(ds) = state.apply(&logged.module, op, &logged.text) {
                return Err(ds
                    .into_iter()
                    .map(|d| {
                        Diag::new(self.span, format!("the log does not replay: {}", d.message))
                    })
                    .collect());
            }
            step.extend(grants_back(&before, &state));
            steps.push(step);
        }
        for step in steps.into_iter().rev() {
            for inv in step {
                let recorded = matches!(
                    inv,
                    Inverse::Ops(_)
                        | Inverse::Derived(_)
                        | Inverse::Unmerge { .. }
                        | Inverse::MoveUp { .. }
                        | Inverse::Replay(_)
                );
                self.undo(inv)?;
                self.src.reconcile_ids();
                if let (Some(steps), false) = (self.steps, recorded) {
                    steps.borrow_mut().push(Step {
                        after: self.src.clone(),
                        maps: Vec::new(),
                    });
                }
            }
        }
        Ok(())
    }

    /// The text of the fill a revert gives for field path `path`.
    pub(super) fn fill_for(&self, fills: &[(ast::FieldPath, Fill)], path: &str) -> Option<Written> {
        fills
            .iter()
            .find(|(p, _)| path_text(p) == path)
            .map(|(_, f)| match f {
                Fill::Expr(e) => self.copy(e.span),
                Fill::Match(arms) => {
                    let start = arms[0].patterns[0].span.start as usize;
                    let end = arms.last().expect("an arm").body.span.end as usize;
                    Written::concat(["match\n  | ".into(), self.copy_range(start..end)])
                }
            })
    }

    /// The step that puts the fields of a part of `path`'s type back in the order `before` declares them.
    pub(super) fn reorder(
        &self,
        before: &Source,
        path: &ast::FieldPath,
        ctor: Option<String>,
    ) -> Inverse {
        let m = self.module_of(&path.ty);
        let order = before
            .find("type", &m, &path.ty.name.text)
            .and_then(|i| match before.parsed(i) {
                Some(Decl::Type(t)) => {
                    let part = match &t.body {
                        ast::TypeBody::Record(_) => Some(t.name.text.clone()),
                        _ => ctor.clone(),
                    };
                    Some(
                        decl_fields(&t)
                            .into_iter()
                            .filter(|(c, _)| c.map(str::to_string) == part)
                            .map(|(_, f)| f.name.text.clone())
                            .collect(),
                    )
                }
                _ => None,
            })
            .unwrap_or_default();
        Inverse::Reorder {
            ty: QName {
                module: Some(ast::Ident {
                    text: m,
                    span: self.span,
                }),
                name: path.ty.name.clone(),
            },
            ctor,
            order,
        }
    }

    /// The text of constructor `c` of type `ty` as `before` declares it, with its field names.
    pub(super) fn before_ctor(
        &self,
        before: &Source,
        ty: &QName,
        c: &str,
    ) -> R<(String, Vec<String>)> {
        let m = self.module_of(ty);
        let missing = || {
            vec![Diag::new(
                self.span,
                "the log does not hold the constructor",
            )]
        };
        let i = before.find("type", &m, &ty.name.text).ok_or_else(missing)?;
        let Some(Decl::Type(t)) = before.parsed(i) else {
            return Err(missing());
        };
        let h = header(&m).len();
        let ctors = ctor_extents(&before.decls[i].text, h, &t);
        let k = ctors.iter().position(|x| x.0 == c).ok_or_else(missing)?;
        let text = before.decls[i].text[ctors[k].1.start..ctors[k].2].to_string();
        let fields = decl_fields(&t)
            .into_iter()
            .filter(|(x, _)| *x == Some(c))
            .map(|(_, f)| f.name.text.clone())
            .collect();
        Ok((text, fields))
    }

    /// The unique and index clauses `before` declares over `ctors` alone, as `in` clauses of an operator.
    pub(super) fn before_clauses(
        &self,
        before: &Source,
        ty: &QName,
        ctors: &[String],
    ) -> Vec<String> {
        let Ok(schema) = before.schema() else {
            return Vec::new();
        };
        let Ok(def) = self.type_id(&schema, ty) else {
            return Vec::new();
        };
        let td = schema.ty(def);
        let ix: Vec<u32> = ctors.iter().filter_map(|c| td.ctor_index(c)).collect();
        let mut out = Vec::new();
        let cx = Cx {
            src: &mut before.clone(),
            module: self.module.clone(),
            text: self.text,
            span: self.span,
            noted: None,
            copying: false,
            steps: None,
        };
        let Ok(clauses) = cx.scoped_clauses(&schema, def) else {
            return Vec::new();
        };
        for cl in clauses {
            let Some(scope) = &cl.scope else { continue };
            if scope.iter().all(|c| ix.contains(c)) {
                let names: Vec<String> = scope
                    .iter()
                    .map(|&c| td.ctors()[c as usize].name.clone())
                    .collect();
                out.push(format!(
                    "\n  {} ({}) in {}",
                    cl.kind,
                    cl.fields.join(", "),
                    sel_text(&names)
                ));
            }
        }
        out
    }

    pub(super) fn split_inverse(
        &self,
        before: &Source,
        logged: &Logged,
        path: &ast::CtorPath,
        into: &[ast::CtorDef],
        arms: &[ast::Arm],
        clauses: &[ast::ArmClause],
    ) -> R<Vec<Inverse>> {
        let lossy = || {
            vec![Diag::new(
                self.span,
                format!(
                    "the split of `{}` routes rows by a routing that is not invertible, so it has no inverse",
                    path.ctor.text
                ),
            )]
        };
        let reached: BTreeSet<&str> = arms
            .iter()
            .filter_map(|a| match &a.body.kind {
                ast::ExprKind::Ctor { path, ctor, .. } => {
                    Some(ctor.as_ref().unwrap_or(&path.name).text.as_str())
                }
                _ => None,
            })
            .collect();
        if let Some(d) = into
            .iter()
            .find(|d| !reached.contains(d.name.text.as_str()))
        {
            return self.err(format!(
                "the split of `{}` routes no row to `{}`, so it is lossy and has no inverse",
                path.ctor.text, d.name.text
            ));
        }
        let (c_text, c_fields) = self.before_ctor(before, &path.ty, &path.ctor.text)?;
        let text = |s: Span| logged.text[s.range()].to_string();
        let mut inverse_arms = Vec::new();
        for a in arms {
            let Some(ast::Pattern {
                kind: ast::PatternKind::Ctor { payload, .. },
                ..
            }) = a.patterns.first()
            else {
                return Err(lossy());
            };
            let ast::ExprKind::Ctor {
                path: bp,
                ctor: bc,
                record,
            } = &a.body.kind
            else {
                return Err(lossy());
            };
            let product = bc.as_ref().unwrap_or(&bp.name).text.clone();
            let given: Vec<(String, &Expr)> = match record {
                Some(ast::RecordExpr::Fields(fs, _)) => {
                    fs.iter().map(|(g, e)| (g.text.clone(), e)).collect()
                }
                None => Vec::new(),
                _ => return Err(lossy()),
            };
            let mut vars: BTreeMap<String, String> = BTreeMap::new();
            let mut fixed: BTreeMap<String, String> = BTreeMap::new();
            let mut binder = None;
            match payload {
                Some(ast::Payload::Record(fs)) => {
                    for f in fs {
                        match &f.pattern {
                            None => {
                                vars.insert(f.field.text.clone(), f.field.text.clone());
                            }
                            Some(ast::Pattern {
                                kind: ast::PatternKind::Bind(x),
                                ..
                            }) => {
                                vars.insert(f.field.text.clone(), x.text.clone());
                            }
                            Some(p) => {
                                fixed.insert(f.field.text.clone(), text(p.span));
                            }
                        }
                    }
                }
                Some(ast::Payload::Bind(x)) => binder = Some(x.text.clone()),
                _ => {}
            }
            let mut out = Vec::new();
            for f in &c_fields {
                if let Some(v) = fixed.get(f) {
                    out.push(format!("{f}: {v}"));
                    continue;
                }
                let target = given.iter().find_map(|(g, e)| match &e.kind {
                    ast::ExprKind::Name(q) if vars.get(f) == Some(&q.name.text) => Some(g.clone()),
                    ast::ExprKind::Field(x, ff) if ff.text == *f => match &x.kind {
                        ast::ExprKind::Name(q) if Some(&q.name.text) == binder.as_ref() => {
                            Some(g.clone())
                        }
                        _ => None,
                    },
                    _ => None,
                });
                match target {
                    Some(g) => out.push(format!("{f}: r.{g}")),
                    None => return Err(lossy()),
                }
            }
            inverse_arms.push(format!(
                "\n    | {product} r -> {} {{ {} }}",
                path.ctor.text,
                out.join(", ")
            ));
        }
        let m = self.module_of(&path.ty);
        let products: Vec<String> = into.iter().map(|d| d.name.text.clone()).collect();
        let mut s = format!(
            "merge constructor {m}::{}.({}) into {c_text}\n  by match{}",
            path.ty.name.text,
            products.join(" | "),
            inverse_arms.concat()
        );
        s.push_str(&self.restored_arms(before, clauses, std::slice::from_ref(&path.ctor.text)));
        Ok(vec![Inverse::Derived(s.into())])
    }

    pub(super) fn merge_inverse(
        &self,
        before: &Source,
        ty: &QName,
        sel: &CtorSel,
        into: &ast::CtorDef,
        arms: Option<&[ast::Arm]>,
        clauses: &[ast::ArmClause],
    ) -> R<Vec<Inverse>> {
        let lossy = || {
            vec![Diag::new(
                self.span,
                format!(
                    "the merge into `{}` computes or drops fields, so it has no inverse",
                    into.name.text
                ),
            )]
        };
        let sources: Vec<String> = sel_idents(sel).iter().map(|x| x.text.clone()).collect();
        let schema = before
            .schema()
            .map_err(|_| vec![Diag::new(self.span, "the log does not replay")])?;
        let module = schema
            .module_named(&self.module_of(ty))
            .unwrap_or(schema::PRELUDE);
        let wanted: BTreeSet<&str> = sources.iter().map(String::as_str).collect();
        let fields = into.record.as_deref().unwrap_or_default();
        let disc = fields
            .iter()
            .find(|f| match &f.ty.kind {
                ast::TypeKind::Named {
                    name,
                    sel: None,
                    args,
                } if args.is_empty() => match schema.lookup_type(module, name) {
                    Ok(schema::TypeName::Decl(d)) => {
                        let st = schema.ty(d);
                        st.ctors().iter().all(|c| c.fields.is_empty())
                            && st
                                .ctors()
                                .iter()
                                .map(|c| c.name.as_str())
                                .collect::<BTreeSet<_>>()
                                == wanted
                    }
                    _ => false,
                },
                _ => false,
            })
            .map(|f| f.name.text.clone())
            .ok_or_else(lossy)?;
        let m_fields: Vec<String> = fields
            .iter()
            .map(|f| f.name.text.clone())
            .filter(|f| *f != disc)
            .collect();
        let mut out_arms = Vec::new();
        let mut products = Vec::new();
        for src in &sources {
            let (text, src_fields) = self.before_ctor(before, ty, src)?;
            products.push(text);
            let mut given = Vec::new();
            for f in &src_fields {
                let g = match arms {
                    None => m_fields.contains(f).then(|| f.clone()),
                    Some(arms) => arms.iter().find_map(|a| {
                        let Some(ast::Pattern {
                            kind:
                                ast::PatternKind::Ctor {
                                    path,
                                    ctor,
                                    payload,
                                },
                            ..
                        }) = a.patterns.first()
                        else {
                            return None;
                        };
                        if ctor.as_ref().unwrap_or(&path.name).text != *src {
                            return None;
                        }
                        let binder = match payload {
                            Some(ast::Payload::Bind(x)) => Some(x.text.clone()),
                            _ => None,
                        };
                        let bound: BTreeMap<String, String> = match payload {
                            Some(ast::Payload::Record(fs)) => fs
                                .iter()
                                .filter_map(|p| match &p.pattern {
                                    None => Some((p.field.text.clone(), p.field.text.clone())),
                                    Some(ast::Pattern {
                                        kind: ast::PatternKind::Bind(x),
                                        ..
                                    }) => Some((x.text.clone(), p.field.text.clone())),
                                    _ => None,
                                })
                                .collect(),
                            _ => BTreeMap::new(),
                        };
                        let ast::ExprKind::Ctor {
                            record: Some(ast::RecordExpr::Fields(fs, _)),
                            ..
                        } = &a.body.kind
                        else {
                            return None;
                        };
                        fs.iter().find_map(|(g, e)| match &e.kind {
                            ast::ExprKind::Field(x, ff) if ff.text == *f => match &x.kind {
                                ast::ExprKind::Name(q) if Some(&q.name.text) == binder.as_ref() => {
                                    Some(g.text.clone())
                                }
                                _ => None,
                            },
                            ast::ExprKind::Name(q) if bound.get(&q.name.text) == Some(f) => {
                                Some(g.text.clone())
                            }
                            _ => None,
                        })
                    }),
                };
                match g {
                    Some(g) => given.push(format!("{f}: r.{g}")),
                    None => return Err(lossy()),
                }
            }
            out_arms.push(format!(
                "\n    | {} {{ {disc}: {src}, {} }} -> {src} {{ {} }}",
                into.name.text,
                m_fields
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
                given
                    .iter()
                    .map(|g| g.replace("r.", ""))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let m = self.module_of(ty);
        let mut s = format!(
            "split constructor {m}::{}.{} into {}\n  by match{}",
            ty.name.text,
            into.name.text,
            products.join(" | "),
            out_arms.concat()
        );
        for c in self.before_clauses(before, ty, &sources) {
            s.push_str(&c);
        }
        s.push_str(&self.restored_arms(before, clauses, &sources));
        Ok(vec![Inverse::Unmerge {
            split: s.into(),
            checks: self.carried_checks_back(before, &schema, ty, &sources)?,
        }])
    }

    /// Returns each check over some of `sources` of type `ty`, by module and name, with its text in
    /// `before`, whose schema is `schema`.
    fn carried_checks_back(
        &self,
        before: &Source,
        schema: &Schema,
        ty: &QName,
        sources: &[String],
    ) -> R<Vec<(String, String, String)>> {
        let def = self.type_id(schema, ty)?;
        let td = schema.ty(def);
        let src_ix: Vec<u32> = sources.iter().filter_map(|s| td.ctor_index(s)).collect();
        let rels: Vec<RelId> = Self::relations_of(schema, def)
            .into_iter()
            .map(|(r, _, _)| r)
            .collect();
        Ok(schema
            .checks
            .iter()
            .filter(|chk| {
                rels.contains(&chk.relation)
                    && chk
                        .ctors
                        .as_ref()
                        .is_some_and(|cs| cs.iter().any(|c| src_ix.contains(c)))
            })
            .map(|chk| {
                let d = &before.decls[chk.file.0 as usize];
                (d.module.clone(), chk.name.clone(), d.text.clone())
            })
            .collect())
    }

    /// The `arm` clauses that put back, for each dependent `clauses` name, its arms over `ctors` in `before`.
    pub(super) fn restored_arms(
        &self,
        before: &Source,
        clauses: &[ast::ArmClause],
        ctors: &[String],
    ) -> String {
        let mut out = String::new();
        let mut seen = BTreeSet::new();
        for a in clauses {
            let target = a.target.name.text.clone();
            if !seen.insert(target.clone()) {
                continue;
            }
            for kind in ["fn", "constraint"] {
                let m = self.module_of(&a.target);
                let Some(i) = before.find(kind, &m, &target) else {
                    continue;
                };
                let Some(d) = before.parsed(i) else { continue };
                let h = header(&m).len();
                for e in decl_exprs(&d) {
                    crate::visit::each_expr(e, &mut |x| {
                        if let ast::ExprKind::Match(_, arms) = &x.kind {
                            for arm in arms {
                                if let Some(ast::Pattern {
                                    kind: ast::PatternKind::Ctor { path, ctor, .. },
                                    ..
                                }) = arm.patterns.first()
                                    && ctors.contains(&ctor.as_ref().unwrap_or(&path.name).text)
                                {
                                    let start = arm.patterns[0].span.start as usize - h;
                                    let end = arm.body.span.end as usize - h;
                                    out.push_str(&format!(
                                        "\n  arm {target}: {}",
                                        &before.decls[i].text[start..end]
                                    ));
                                }
                            }
                        }
                    });
                }
            }
        }
        out
    }

    /// Set type `ty`, a sum of one constructor named after it, to record text `text`, its header
    /// fields keeping their ids as fields of that constructor.
    fn unsum(&mut self, ty: &QName, text: String) -> R<()> {
        let (i, _) = self.type_decl(ty)?;
        self.src.rewrite(i, text);
        let header = format!("field {}::{}.", self.module_of(ty), ty.name.text);
        let ids = std::mem::take(&mut self.src.ids);
        self.src.ids = ids
            .into_iter()
            .map(|(k, id)| match k.strip_prefix(&header) {
                Some(f) if !f.contains('.') => (format!("{header}{}.{f}", ty.name.text), id),
                _ => (k, id),
            })
            .collect();
        Ok(())
    }

    /// Apply operators given as source text in the migration's module as `how` says, then set
    /// each check of `checks`, by module and name, to its text within the last operator's step.
    fn undo_ops(&mut self, text: &Written, how: How, checks: &[(String, String, String)]) -> R<()> {
        let span = self.span;
        let wrap = |ds: Vec<Diag>, what: &str| -> Vec<Diag> {
            ds.into_iter()
                .map(|d| {
                    Diag::new(
                        span,
                        format!("the inverse `{what}` does not apply: {}", d.message),
                    )
                })
                .collect()
        };
        let head = format!("module {}\n", self.module);
        let full = format!("{head}{}", text.text);
        let m = crate::parse::parse_migration(&full, Options::default())
            .ok()
            .map_err(|ds| wrap(ds, &text.text))?;
        let noted = RefCell::new(Names::default());
        for op in &m.operators {
            let before = self.src.clone();
            let mut sub = Cx {
                src: &mut *self.src,
                module: self.module.clone(),
                text: &full,
                span: op.span,
                noted: self.noted.map(|_| &noted),
                copying: false,
                steps: None,
            };
            match (&op.kind, how) {
                (
                    OperatorKind::MergeConstructor {
                        ty,
                        sel,
                        into,
                        arms,
                        clauses,
                    },
                    How::Derived,
                ) => sub.merge(ty, sel, into, arms.as_deref(), clauses, true),
                (OperatorKind::MoveField { path, fill, .. }, How::UpFrom(from)) => {
                    sub.move_up(path, from, fill.as_ref())
                }
                (kind, _) => sub.operator(kind),
            }
            .map_err(|ds| wrap(ds, &full[op.span.range()]))?;
            self.src.carry = None;
            self.src.rename_ids(&before, &self.module, &op.kind);
            self.src.reconcile_ids();
            if let Some(steps) = self.steps {
                let maps = before
                    .row_maps(&self.module, op, &full, self.src, how)
                    .map_err(|ds| wrap(ds, &full[op.span.range()]))?;
                steps.borrow_mut().push(Step {
                    after: self.src.clone(),
                    maps,
                });
            }
        }
        for (module, name, check) in checks {
            if let Some(i) = self.src.find("check", module, name) {
                self.src.rewrite(i, check.clone());
            }
        }
        self.src.reconcile_ids();
        if let (Some(steps), false) = (self.steps, checks.is_empty())
            && let Some(last) = steps.borrow_mut().last_mut()
        {
            last.after = self.src.clone();
        }
        if let Some(out) = self.noted {
            let noted = noted.into_inner();
            for (o, from) in &text.copies {
                let at = head.len() + o;
                out.borrow_mut()
                    .copy(&noted, at..at + from.len(), from.start);
            }
        }
        Ok(())
    }

    /// Apply one step of an inverse to the source.
    pub(super) fn undo(&mut self, inv: Inverse) -> R<()> {
        match inv {
            Inverse::Ops(text) => self.undo_ops(&text, How::Written, &[]),
            Inverse::Derived(text) => self.undo_ops(&text, How::Derived, &[]),
            Inverse::Unmerge { split, checks } => self.undo_ops(&split, How::Written, &checks),
            Inverse::MoveUp { text, from } => self.undo_ops(&text, How::UpFrom(&from), &[]),
            Inverse::Unadd { path, record } => {
                self.remove_ctor(&path, true)?;
                if let Some(text) = record {
                    self.unsum(&path.ty, text)?;
                }
                Ok(())
            }
            Inverse::Readd {
                ty,
                text,
                after,
                arms,
            } => self.readd(&ty, &text, after.as_deref(), arms),
            Inverse::States(states) => {
                for (relation, key, state) in states {
                    let map = if relation {
                        &mut self.src.states
                    } else {
                        &mut self.src.value_states
                    };
                    match state {
                        Some(s) => {
                            map.insert(key, s);
                        }
                        None => {
                            map.remove(&key);
                        }
                    }
                }
                Ok(())
            }
            Inverse::Grants { back, ids } => {
                self.src.hold(back, &ids);
                Ok(())
            }
            Inverse::Decl {
                module,
                kind,
                name,
                text,
                states,
            } => self.restore_decl(&module, kind, &name, text, states),
            Inverse::Module {
                name,
                present,
                installed,
                decls,
            } => self.restore_module(name, present, installed, decls),
            Inverse::Reorder { ty, ctor, order } => {
                self.reorder_fields(&ty, ctor.as_deref(), &order)
            }
            Inverse::KeyClause { relation, text } => {
                let (i, rd, ranges) = self.clauses(&relation)?;
                if let Some(k) = rd
                    .clauses
                    .iter()
                    .position(|c| matches!(c, RelationClause::Key(..)))
                {
                    self.src.edit(i, vec![(ranges[k].clone(), text)]);
                }
                Ok(())
            }
            Inverse::Replay(l) => {
                self.src.expecting(&l.module, &l.operators);
                for op in &l.operators {
                    match self.steps {
                        Some(steps) => {
                            let mut more = self.src.steps(&l.module, op, &l.text)?;
                            steps.borrow_mut().append(&mut more);
                        }
                        None => self.src.apply(&l.module, op, &l.text)?,
                    }
                }
                self.src.settled();
                Ok(())
            }
        }
    }

    fn readd(
        &mut self,
        ty: &QName,
        text: &str,
        after: Option<&str>,
        arms: Vec<(String, Vec<String>)>,
    ) -> R<()> {
        let (i, t) = self.type_decl(ty)?;
        let h = header(&self.src.decls[i].module).len();
        let dtext = self.src.decls[i].text.clone();
        let ctors = ctor_extents(&dtext, h, &t);
        let edit = match after
            .as_ref()
            .and_then(|a| ctors.iter().find(|c| c.0 == *a))
        {
            Some(c) => (
                c.2..c.2,
                format!(
                    "
  | {text}"
                ),
            ),
            None => match ctors.first() {
                Some(c) => (
                    c.1.start..c.1.start,
                    format!(
                        "{text}
  | "
                    ),
                ),
                None => return self.err("the type has no constructors to place it beside"),
            },
        };
        self.src.edit(i, vec![edit]);
        for (name, texts) in arms {
            let m = self.module.clone();
            let Some(k) = self.src.find("constraint", &m, &name) else {
                continue;
            };
            let schema = self.schema()?;
            let def = self.type_id(&schema, ty)?;
            let refs = check::schema_refs(&schema);
            let Some((_, at, _)) = refs.iter().find(|(f, _, r)| {
                f.0 as usize == k && matches!(r, Ref::Match(x) if holds(&schema, x, def, 0))
            }) else {
                continue;
            };
            let end = at.end as usize - header(&m).len();
            let s: String = texts
                .iter()
                .map(|a| {
                    format!(
                        "
  | {a}"
                    )
                })
                .collect();
            self.src.edit(k, vec![(end..end, s)]);
        }
        Ok(())
    }

    fn restore_decl(
        &mut self,
        module: &str,
        kind: &str,
        name: &str,
        text: Option<String>,
        states: Vec<(StateKey, State)>,
    ) -> R<()> {
        let found = self.src.find(kind, module, name);
        match (found, text) {
            (Some(i), Some(t)) => self.src.rewrite(i, t),
            (Some(i), None) => {
                self.src.remove_decl(i);
                if kind == "relation" {
                    self.src
                        .states
                        .retain(|(m, r, _), _| !(*m == module && *r == name));
                }
                if kind == "type" {
                    self.src
                        .value_states
                        .retain(|(m, t, _), _| !(*m == module && *t == name));
                }
            }
            (None, Some(t)) => {
                self.src.push(module, t);
            }
            (None, None) => {}
        }
        for (k, v) in states {
            self.src.states.insert(k, v);
        }
        Ok(())
    }

    fn restore_module(
        &mut self,
        name: String,
        present: bool,
        installed: bool,
        decls: Vec<String>,
    ) -> R<()> {
        if present {
            self.src.modules.insert(name.clone());
            for d in decls {
                self.src.push(&name, d);
            }
            if installed {
                self.src.installed.insert(name);
            }
        } else if installed {
            self.uninstall(&ast::Ident {
                text: name,
                span: self.span,
            })?;
        } else {
            if self.src.decls.iter().any(|d| d.module == name) {
                return self.err(format!(
                    "`drop module {name}` is refused while the module holds a declaration"
                ));
            }
            self.src.modules.remove(&name);
            self.src.installed.remove(&name);
        }
        Ok(())
    }

    fn reorder_fields(&mut self, ty: &QName, ctor: Option<&str>, order: &[String]) -> R<()> {
        let (i, t) = self.type_decl(ty)?;
        let h = header(&self.src.decls[i].module).len();
        let text = self.src.decls[i].text.clone();
        let Some(braces) = field_braces(&text, h, &t, ctor) else {
            return Ok(());
        };
        let part = match (&t.body, &ctor) {
            (ast::TypeBody::Record(_), _) => Part::Ctor(t.name.text.clone()),
            (_, Some(c)) => Part::Ctor(c.to_string()),
            (_, None) => Part::Header,
        };
        let mut items: Vec<(String, String)> = Vec::new();
        for (c, f) in decl_fields(&t) {
            let here = match (&part, c) {
                (Part::Header, None) => true,
                (Part::Ctor(p), Some(c)) => p == c,
                _ => false,
            };
            if here {
                let start = f.name.span.start as usize - h;
                let end = item_end(&text, start);
                items.push((f.name.text.clone(), text[start..end].trim_end().to_string()));
            }
        }
        if items.is_empty() {
            return Ok(());
        }
        items.sort_by_key(|(n, _)| order.iter().position(|o| o == n).unwrap_or(usize::MAX));
        let inner: Vec<String> = items.into_iter().map(|(_, t)| t).collect();
        self.src
            .edit(i, vec![(braces, format!("{{ {} }}", inner.join(", ")))]);
        Ok(())
    }
}

/// A field path with its type qualified by `module`.
pub(super) fn qualified_path(p: &ast::FieldPath, module: &str) -> String {
    match &p.ctor {
        Some(c) => format!("{module}::{}.{}.{}", p.ty.name.text, c.text, p.field.text),
        None => format!("{module}::{}.{}", p.ty.name.text, p.field.text),
    }
}

/// The type text of the field a path names in the schema `src`, its delete action included.
pub(super) fn field_type_text(src: &Source, module: &str, p: &ast::FieldPath) -> Option<String> {
    let (ty, action) = field_texts(src, module, p)?;
    Some(match action {
        Some(a) => format!("{ty} on delete {a}"),
        None => ty,
    })
}

/// The type text and the delete action text of the field a path names in the schema `src`.
pub(super) fn field_texts(
    src: &Source,
    module: &str,
    p: &ast::FieldPath,
) -> Option<(String, Option<String>)> {
    let i = src.find("type", module, &p.ty.name.text)?;
    let Some(Decl::Type(t)) = src.parsed(i) else {
        return None;
    };
    let h = header(module).len();
    decl_fields(&t).into_iter().find_map(|(c, f)| {
        let here = match (&p.ctor, c) {
            (None, None) => true,
            (None, Some(c)) => matches!(t.body, ast::TypeBody::Record(_)) && c == t.name.text,
            (Some(x), Some(c)) => x.text == c,
            (Some(_), None) => false,
        };
        (here && f.name.text == p.field.text).then(|| {
            let text = &src.decls[i].text;
            let local = |s: Span| text[s.start as usize - h..s.end as usize - h].to_string();
            (
                local(f.ty.span),
                f.on_delete.as_ref().map(|a| local(a.span)),
            )
        })
    })
}
