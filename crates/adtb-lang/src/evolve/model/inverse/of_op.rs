use std::cell::OnceCell;

use super::*;

/// The names of a migration's text, each operand naming an object tied to its id, computed once.
pub(super) type Spelling = OnceCell<Replaying<Names>>;

/// An operator of migration `logged` being inverted: `op` as written and `resolved` as the model
/// resolved it against `before_m`, the model before it, which leaves `after_m`; `before` and
/// `after` are their sources, and `start` the last source at or before `before` that builds.
#[derive(Clone, Copy)]
pub(super) struct Inverting<'a> {
    before: &'a Source,
    after: &'a Source,
    start: &'a Source,
    before_m: &'a Model,
    after_m: &'a Model,
    logged: &'a Logged,
    spelling: &'a Spelling,
    op: &'a Operator,
    resolved: &'a Op,
    fills: &'a Fills<'a>,
}

/// The parts of [`Inverting`] one logged migration's operators share.
#[derive(Clone, Copy)]
pub(super) struct Run<'a> {
    logged: &'a Logged,
    spelling: &'a Spelling,
    fills: &'a Fills<'a>,
}

/// A source with the model of it.
pub(super) type Modelled<'a> = (&'a Source, &'a Model);

impl<'a> Run<'a> {
    pub(super) fn new(logged: &'a Logged, spelling: &'a Spelling, fills: &'a Fills<'a>) -> Self {
        Run {
            logged,
            spelling,
            fills,
        }
    }

    /// Returns the inversion of `op`, resolved as `resolved`, from `before` to `after`, with
    /// `start` the last source at or before `before` that builds.
    pub(super) fn at(
        self,
        (before, after): (Modelled<'a>, Modelled<'a>),
        start: &'a Source,
        op: &'a Operator,
        resolved: &'a Op,
    ) -> Inverting<'a> {
        Inverting {
            before: before.0,
            after: after.0,
            start,
            before_m: before.1,
            after_m: after.1,
            logged: self.logged,
            spelling: self.spelling,
            op,
            resolved,
            fills: self.fills,
        }
    }
}

/// Returns the refusal of a revert whose log does not resolve, at `span`.
fn unresolved(span: Span, ds: &[Diag]) -> Vec<Diag> {
    ds.iter()
        .map(|d| Diag::new(span, format!("the log does not resolve: {}", d.message)))
        .collect()
}

impl<'a> Inverting<'a> {
    pub(super) fn before(&self) -> &'a Source {
        self.before
    }

    pub(super) fn after(&self) -> &'a Source {
        self.after
    }

    pub(super) fn op(&self) -> &'a Operator {
        self.op
    }

    pub(super) fn fills(&self) -> &'a Fills<'a> {
        self.fills
    }

    pub(super) fn before_m(&self) -> &'a Model {
        self.before_m
    }

    pub(super) fn after_m(&self) -> &'a Model {
        self.after_m
    }
}

impl Inverting<'_> {
    /// Returns the id of the one index the operator adds, when the sources before and after it
    /// build.
    fn added_index(&self) -> Option<u64> {
        let ids = |src: &Source| -> Option<BTreeSet<u64>> {
            Some(
                src.schema()
                    .ok()?
                    .ids()
                    .index_ids()
                    .map(|(_, id)| id.raw())
                    .collect(),
            )
        };
        let (a, b) = (ids(self.before)?, ids(self.after)?);
        match b.difference(&a).collect::<Vec<_>>().as_slice() {
            [id] => Some(**id),
            _ => None,
        }
    }

    pub(super) fn of(&self) -> String {
        self.text(self.op.span)
    }

    /// Returns `range` of the migration's text, each name of an object in it held by its id.
    pub(super) fn op_text(&self, range: Range<usize>) -> R<Term> {
        let names = self
            .spelling
            .get_or_init(|| self.logged.names())
            .as_ref()
            .map_err(|f| f.clone().map_refused(|ds| unresolved(self.op.span, &ds)))?;
        let mut out = Term::default();
        out.copy(self.logged.text(), range, names, 0);
        Ok(out)
    }

    /// Returns the migration's text from `q` to `end`, `q` qualified by its module.
    pub(super) fn qualified(&self, q: &QName, end: usize) -> R<Term> {
        let mut out = match &q.module {
            Some(_) => Term::default(),
            None => Term::from(format!("{}::", self.logged.module())),
        };
        let start = q
            .module
            .as_ref()
            .map_or(q.name.span.start, |m| m.span.start);
        out.append(self.op_text(start as usize..end)?);
        Ok(out)
    }

    /// Returns field path `p` qualified by its module.
    fn path(&self, p: &ast::FieldPath) -> R<Term> {
        self.qualified(&p.ty, p.field.span.end as usize)
    }

    /// Returns field `f` of the model before the operator as a path qualified by its module,
    /// each part held by its id.
    fn field_path(&self, f: u64) -> R<Term> {
        let m = self.before_m;
        let missing = || vec![Diag::new(self.op.span, "the log does not hold the field")];
        let field = m.fields.get(&f).ok_or_else(missing)?;
        let t = m.field_type(f).ok_or_else(missing)?;
        let mut out = self.type_term(t)?;
        if let Some(c) = m.ctors.get(&field.owner)
            && !matches!(m.types[&t].body, TypeBody::Record { .. })
        {
            out.push(".");
            out.push_name(&c.name, Space::Shared, Some(field.owner));
        }
        out.push(".");
        out.push_name(&field.name, Space::Field, Some(f));
        Ok(out)
    }

    /// Returns type `t` of the model before the operator qualified by its module, held by its id.
    fn type_term(&self, t: u64) -> R<Term> {
        let (m, name) = self.type_named(t)?;
        let mut out = Term::from(format!("{m}::"));
        out.push_name(&name, Space::Shared, Some(t));
        Ok(out)
    }

    /// Returns relation variable `r` of the model before the operator qualified by its module,
    /// held by its id.
    fn relation_term(&self, r: u64) -> R<Term> {
        let m = self.before_m;
        let missing = || {
            vec![Diag::new(
                self.op.span,
                "the log does not hold the relation",
            )]
        };
        let name = &m.relations.get(&r).ok_or_else(missing)?.name;
        let module = m.module_of((Space::Shared, r)).ok_or_else(missing)?;
        let mut out = Term::from(format!("{module}::"));
        out.push_name(name, Space::Shared, Some(r));
        Ok(out)
    }

    /// Returns the module and name of type `t` in the model before the operator.
    fn type_named(&self, t: u64) -> R<(String, String)> {
        let m = self.before_m;
        let missing = || vec![Diag::new(self.op.span, "the log does not hold the type")];
        let name = m.types.get(&t).ok_or_else(missing)?.name.clone();
        let module = m.module_of((Space::Shared, t)).ok_or_else(missing)?;
        Ok((module.to_string(), name))
    }

    /// Returns type `t` of the model before the operator as a name qualified by its module.
    pub(super) fn type_qname(&self, t: u64) -> R<QName> {
        let (m, name) = self.type_named(t)?;
        let ident = |text: String| ast::Ident {
            text,
            span: Span::default(),
        };
        Ok(QName {
            module: Some(ident(m)),
            name: ident(name),
        })
    }

    /// Returns field `f` of the model before the operator as a path of names qualified by its
    /// module.
    fn field_qpath(&self, f: u64) -> R<ast::FieldPath> {
        let m = self.before_m;
        let missing = || vec![Diag::new(self.op.span, "the log does not hold the field")];
        let field = m.fields.get(&f).ok_or_else(missing)?;
        let t = m.field_type(f).ok_or_else(missing)?;
        let ident = |text: &str| ast::Ident {
            text: text.to_string(),
            span: Span::default(),
        };
        let ctor = match m.ctors.get(&field.owner) {
            Some(c) if !matches!(m.types[&t].body, TypeBody::Record { .. }) => Some(ident(&c.name)),
            _ => None,
        };
        Ok(ast::FieldPath {
            ty: self.type_qname(t)?,
            ctor,
            field: ident(&field.name),
        })
    }

    /// Returns `range` of the text of declaration `i` of `before`, each name of an object in it
    /// held by its id, as `before` resolves it, else as the source at the start of the run does
    /// when it holds the same text and `before` does not build.
    pub(super) fn decl_range(&self, i: usize, range: Range<usize>) -> R<Term> {
        let d = &self.before.decls[i];
        let names = match self.before.names() {
            Ok(mut all) => all.swap_remove(i),
            Err(_) => {
                let j = self
                    .start
                    .decls
                    .iter()
                    .position(|s| s.module == d.module && s.text == d.text);
                match (j, self.start.names()) {
                    (Some(j), Ok(mut all)) => all.swap_remove(j),
                    _ => {
                        return Err(vec![Diag::new(
                            self.op.span,
                            "the log does not resolve the names of the declaration the operator changes",
                        )]
                        .into());
                    }
                }
            }
        };
        let mut out = Term::default();
        out.copy(&d.text, range, &names, header(&d.module).len());
        Ok(out)
    }

    /// Returns the operator's text after its first word, `verb` in its place and the relation of
    /// `on` qualified by its module.
    fn clause(&self, verb: &str, on: &ast::RelationRef) -> R<Term> {
        let span = self.op.span.range();
        let text = &self.logged.text()[span.clone()];
        let word = text.find(char::is_whitespace).unwrap_or(text.len());
        let rest = span.start + text.len() - text[word..].trim_start().len();
        let q = &on.relation;
        let rel = q
            .module
            .as_ref()
            .map_or(q.name.span.start, |m| m.span.start) as usize;
        let mut out = Term::from(format!("{verb} "));
        out.append(self.op_text(rest..rel)?);
        out.append(self.qualified(q, span.end)?);
        Ok(out)
    }

    /// Returns the whole text of declaration `i` of `before` as [`Inverting::decl_range`] does.
    pub(super) fn decl(&self, i: usize) -> R<Term> {
        self.decl_range(i, 0..self.before.decls[i].text.len())
    }

    /// Returns the step adding back declaration `item` of the model before the operator, which
    /// the operator removes, with the write states it held: a relation variable's with its named
    /// sets, typed when the migration also drops the type its declaration names.
    fn restore(&self, item: Item) -> R<Vec<Inverse>> {
        let m = self.before_m;
        let i = m.decl_at(item).ok_or_else(|| {
            vec![Diag::new(
                self.op.span,
                "the log does not hold the declaration the operator removes",
            )]
        })?;
        let module = m.decls[i].module.clone();
        let name = m.decl_name(item);
        let text = self.decl(i)?;
        let of = self.of();
        match item {
            Item::Relation(r) => {
                let typed = self.before.relation_type(&module, &name).is_some_and(|t| {
                    self.logged.operators().iter().any(|op| {
                        matches!(&op.kind, OperatorKind::DropType(q)
                            if self.module_of(q) == t.0 && q.name.text == t.1)
                    })
                });
                let mut out = vec![Inverse::RestoreRelation {
                    module,
                    name,
                    text,
                    of: of.clone(),
                    typed,
                }];
                for (j, d) in m.decls.iter().enumerate() {
                    if let Item::Set(s) = d.item
                        && m.sets[&s].relation == r
                    {
                        out.push(Inverse::Restore {
                            module: d.module.clone(),
                            kind: "set",
                            name: m.decl_name(d.item),
                            text: self.decl(j)?,
                            of: of.clone(),
                        });
                    }
                }
                out.push(self.states_back(|s| matches!(s, StateKey::Relation(x, _) if x == r)));
                Ok(out)
            }
            Item::Type(t) => {
                let ctors = m.ctors_of(t);
                Ok(vec![
                    Inverse::Restore {
                        module,
                        kind: "type",
                        name,
                        text,
                        of,
                    },
                    self.states_back(|s| matches!(s, StateKey::Value(c) if ctors.contains(&c))),
                ])
            }
            Item::Set(s) => Ok(vec![self.dropped_set(s)?]),
            Item::Fn(_)
            | Item::Aggregate(_)
            | Item::Constraint(_)
            | Item::Check(_)
            | Item::Store(_)
            | Item::Held(..) => Ok(vec![Inverse::Restore {
                module,
                kind: OpKind::of(item).word(),
                name,
                text,
                of,
            }]),
        }
    }

    /// Returns the step removing the declaration of kind `kind` and name `name` the operator
    /// adds, by its id in the model after it.
    fn unadd(&self, kind: OpKind, name: &str) -> R<Inverse> {
        let missing = || {
            vec![Diag::new(
                self.op.span,
                "the log does not hold the object the operator adds",
            )]
        };
        let m = self.after_m;
        let item = m
            .find(self.logged.module(), kind, name)
            .ok_or_else(missing)?;
        let key_kind = match kind {
            OpKind::Type => KeyKind::Type,
            OpKind::Relation => KeyKind::Relation,
            OpKind::Set => KeyKind::Set,
            OpKind::Fn | OpKind::Aggregate | OpKind::Store => KeyKind::Function,
            OpKind::Constraint | OpKind::Check => KeyKind::Constraint,
        };
        let held = || {
            let key = ObjectKey::named(
                key_kind,
                Owner::Module(self.logged.module().to_string()),
                name,
            );
            self.after.ids.get(&key).copied().ok_or_else(missing)
        };
        let decl = match item {
            Item::Set(s) => {
                return Ok(Inverse::DropSet {
                    set: s,
                    of: self.of(),
                });
            }
            Item::Store(v) => DeclOf::Store(m.stores[&v].function),
            Item::Held(OpKind::Store, _) => DeclOf::Store(held()?),
            Item::Held(..) => DeclOf::Object(key_kind, held()?),
            Item::Type(x)
            | Item::Relation(x)
            | Item::Fn(x)
            | Item::Aggregate(x)
            | Item::Constraint(x)
            | Item::Check(x) => DeclOf::Object(key_kind, x),
        };
        Ok(Inverse::Redeclare {
            decl,
            text: None,
            of: self.of(),
        })
    }

    /// Returns the module `n` names, else the migration's.
    pub(super) fn module_of(&self, n: &QName) -> String {
        n.module
            .as_ref()
            .map_or(self.logged.module().to_string(), |m| m.text.clone())
    }

    fn text(&self, s: Span) -> String {
        self.logged.text()[s.range()].to_string()
    }

    /// Returns the inverse renaming object `obj` back to the name it has before the operator.
    fn renamed(&self, obj: Obj) -> R<Inverse> {
        let to = self.before_m.name(obj).ok_or_else(|| {
            vec![Diag::new(
                self.op.span,
                "the log does not hold the object the operator names",
            )]
        })?;
        let m = self.before_m;
        let word = match obj {
            (Space::Shared, x) if m.types.contains_key(&x) => "type",
            (Space::Shared, x) if m.ctors.contains_key(&x) => "constructor",
            (Space::Shared, _) => "relation",
            (Space::Set, _) => "set",
            _ => "field",
        };
        Ok(Inverse::Rename {
            object: obj,
            word,
            to: to.to_string(),
            of: self.of(),
        })
    }

    /// Returns the inverse adding back named set `s` of the model before the operator, which the
    /// operator drops.
    fn dropped_set(&self, s: u64) -> R<Inverse> {
        let set = self.before_m.sets.get(&s).ok_or_else(|| {
            vec![Diag::new(
                self.op.span,
                "the log does not hold the object the operator names",
            )]
        })?;
        let members = set
            .members
            .iter()
            .filter_map(|t| t.objects().find(|o| o.0 == Space::Shared).map(|o| o.1))
            .collect();
        Ok(Inverse::AddSet {
            relation: set.relation,
            name: set.name.clone(),
            members,
            of: self.of(),
        })
    }

    /// Returns the step setting back each write state the model before the operator holds that
    /// `pick` takes.
    pub(super) fn states_back(&self, pick: impl Fn(StateKey) -> bool) -> Inverse {
        let m = self.before_m;
        let states = m
            .states
            .iter()
            .map(|(&(r, c), &v)| (StateKey::Relation(r, c), Some(v)))
            .chain(
                m.value_states
                    .iter()
                    .map(|(&c, &v)| (StateKey::Value(c), Some(v))),
            )
            .filter(|(s, _)| pick(*s))
            .collect();
        Inverse::States {
            states,
            of: self.text(self.op.span),
        }
    }

    /// Returns the write state `key` names in the model before the operator.
    fn state(&self, key: StateKey) -> Option<State> {
        match key {
            StateKey::Relation(r, c) => self.before_m.states.get(&(r, c)).copied(),
            StateKey::Value(c) => self.before_m.value_states.get(&c).copied(),
        }
    }
}

/// Returns the one step applying `text` as the inverse of operator text `of`.
fn apply(text: Term, of: String) -> R<Vec<Inverse>> {
    Ok(vec![Inverse::Apply { text, of }])
}

impl Cx<'_> {
    /// Returns the inverse of the operator `at` inverts.
    pub(super) fn inverse(&self, at: &Inverting) -> R<Vec<Inverse>> {
        let at = *at;
        let of = at.of();
        let module =
            |name: &ast::Ident, present: bool, installed: bool, decls: Vec<Term>| Inverse::Module {
                name: name.text.clone(),
                present,
                installed,
                decls,
                of: of.clone(),
            };
        match at.resolved {
            Op::AddModule(m) => Ok(vec![module(m, false, false, Vec::new())]),
            Op::Install { name, .. } => Ok(vec![module(name, false, true, Vec::new())]),
            Op::DropModule(m) | Op::Uninstall(m) => {
                let decls = (0..at.before.decls.len())
                    .filter(|&i| at.before.decls[i].module == m.text)
                    .map(|i| at.decl(i))
                    .collect::<R<Vec<Term>>>()?;
                let installed = at.before.installed.contains(&m.text);
                Ok(vec![module(m, true, installed, decls)])
            }
            Op::Add { kind, name, .. } => Ok(vec![at.unadd(*kind, name)?]),
            Op::RedefineFn { function, .. } => self.redefined_inverse(at, *function),
            Op::Drop(item) => at.restore(*item),
            Op::Rename { obj, .. } => Ok(vec![at.renamed(*obj)?]),
            Op::Stage { key, .. } => Ok(vec![Inverse::States {
                states: vec![(*key, at.state(*key))],
                of,
            }]),
            Op::AddField { part, name, .. } => {
                let f = at
                    .after_m
                    .field_of(part.0, part.1, &name.text)
                    .ok_or_else(|| vec![Diag::new(self.span, "the log does not hold the field")])?;
                let mut text = Term::from("drop field ");
                text.append(at.added_field_path(f)?);
                apply(text, of)
            }
            Op::DropField(f) => self.dropped_field_inverse(at, *f),
            Op::AlterField { field, .. } => {
                let mut text = Term::from("alter field ");
                text.append(at.field_path(*field)?);
                text.push(" on delete ");
                match &at.before_m.fields[field].action {
                    Some(a) => text.append(a.clone()),
                    None => text.push("restrict"),
                }
                apply(text, of)
            }
            Op::AddIndex { kind, on, .. } => match (kind, at.added_index()) {
                (IndexKind::Unique, _) | (_, None) => apply(at.clause("drop", on)?, of),
                (_, Some(id)) => Ok(vec![Inverse::Restructure {
                    drops: vec![id],
                    op: Term::default(),
                    adds: Term::default(),
                    how: Undo::Written,
                    checks: Vec::new(),
                    of,
                }]),
            },
            Op::DropIndex { on, .. } => apply(at.clause("add", on)?, of),
            Op::Rekey {
                relation, carry, ..
            } => self.rekey_inverse(at, *relation, carry),
            Op::AddDefault {
                relation,
                ctor,
                field,
                ..
            } => {
                let mut text = Term::from("drop default ");
                text.append(at.default_path(*relation, ctor.as_ref(), field)?);
                apply(text, of)
            }
            Op::DropDefault {
                relation,
                ctor,
                field,
            } => self.default_inverse(at, *relation, ctor.as_ref(), field),
            Op::AddComputed {
                relation, field, ..
            } => {
                let mut text = Term::from("drop computed ");
                text.append(at.default_path(*relation, None, field)?);
                apply(text, of)
            }
            Op::RedefineComputed {
                relation, field, ..
            }
            | Op::DropComputed { relation, field } => self.computed_inverse(at, *relation, field),
            Op::AddConstructor { ty, .. } => self.added_ctor_inverse(&at, *ty),
            Op::RemoveConstructor { ty, ctor } => self.removed_ctor_inverse(&at, *ty, *ctor),
            Op::Split {
                path,
                into,
                arms,
                clauses,
                ..
            } => {
                let arms_of: Vec<ast::ArmClause> = clauses
                    .iter()
                    .filter_map(|c| match c {
                        ast::SplitClause::Arm(a) => Some((**a).clone()),
                        _ => None,
                    })
                    .collect();
                self.split_inverse(&at, path, into, arms, &arms_of)
            }
            Op::Merge {
                ty,
                sel,
                into,
                arms,
                clauses,
            } => self.merge_inverse(
                &at,
                &at.type_qname(*ty)?,
                sel,
                into,
                arms.as_deref(),
                clauses,
            ),
            Op::Nest {
                ty,
                sel,
                into,
                clauses,
            } => self.nest_inverse(&at, &at.type_qname(*ty)?, sel, into, clauses),
            Op::Unnest { path, clauses, .. } => self.unnest_inverse(&at, path, clauses),
            Op::MoveField {
                field, path, sel, ..
            } => self.move_inverse(&at, (*field, path), sel.as_ref()),
            Op::Grant(_) | Op::Revoke(_) | Op::AdoptTzdb { .. } => Ok(Vec::new()),
            Op::Script(_) => self.err("a `script` has no inverse"),
            Op::Revert { migration, .. } => self.replay_inverse(at, migration),
        }
    }

    /// Returns the inverse of redefining function `function`: declaring it as it was.
    fn redefined_inverse(&self, at: Inverting, function: u64) -> R<Vec<Inverse>> {
        let i = at.before_m.decl_at(Item::Fn(function)).ok_or_else(|| {
            vec![Diag::new(
                self.span,
                "the log does not hold the redefined function",
            )]
        })?;
        Ok(vec![Inverse::Redeclare {
            decl: DeclOf::Object(KeyKind::Function, function),
            text: Some(at.decl(i)?),
            of: at.of(),
        }])
    }

    /// Returns the inverse of reverting `migration`: applying its operators again.
    fn replay_inverse(&self, at: Inverting, migration: &str) -> R<Vec<Inverse>> {
        let Some(k) = self.src.logged_at(migration) else {
            return self.err(format!("migration {migration} is not in the log"));
        };
        let logged = &self.src.log[k];
        let names = logged
            .names()
            .map_err(|f| f.map_refused(|ds| unresolved(at.op.span, &ds)))?;
        let texts = logged
            .operators()
            .iter()
            .map(|o| {
                let mut text = Term::from(format!("module {}\n", logged.module()));
                text.copy(logged.text(), o.span.range(), &names, 0);
                text
            })
            .collect();
        Ok(vec![Inverse::Replay {
            logged: Box::new(logged.clone()),
            texts,
            again: self.src.recreated_by(k, at.before, at.after),
            of: at.of(),
        }])
    }

    /// Returns the inverse of dropping field `f`: adding it back with the revert's fill for it,
    /// then putting the fields of its part back in their order.
    fn dropped_field_inverse(&self, at: Inverting, f: u64) -> R<Vec<Inverse>> {
        let path = at.field_qpath(f)?;
        let (m, t) = (at.module_of(&path.ty), &path.ty.name.text);
        let c = path.ctor.as_ref().map(|c| c.text.as_str());
        let key = self.named_now(at.before, &m, t, c, &path.field.text);
        let Some(fill) = self.fill_for(at.fills, at.before, Some(f)) else {
            return self.err(format!(
                "the inverse of `drop field {}` needs a fill: `fill {key} = e`",
                path_text(&path),
            ));
        };
        let field = &at.before_m.fields[&f];
        let mut text = Term::from("add field ");
        text.append(at.field_path(f)?);
        text.push(": ");
        text.append(field.ty.clone());
        if let Some(a) = &field.action {
            text.push(" on delete ");
            text.append(a.clone());
        }
        text.push(" fill ");
        text.push(fill);
        Ok(vec![Inverse::Apply { text, of: at.of() }, at.reorder(f)])
    }

    fn rekey_inverse(&self, at: Inverting, rel: u64, carry: &[ast::FieldPath]) -> R<Vec<Inverse>> {
        let Inverting { before, .. } = at;
        let missing = || vec![Diag::new(self.span, "the log does not hold the key")];
        let i = at
            .before_m
            .decl_at(Item::Relation(rel))
            .ok_or_else(missing)?;
        let m = before.decls[i].module.clone();
        let Some(Decl::Relation(rd)) = before.parsed(i) else {
            return Err(missing().into());
        };
        let ranges = clause_ranges(header(&m).len(), &rd);
        let Some((k, fields)) = rd.clauses.iter().enumerate().find_map(|(k, c)| match c {
            RelationClause::Key(fields, _) => Some((k, fields)),
            _ => None,
        }) else {
            return Err(missing().into());
        };
        let mut text = Term::from("rekey relation ");
        text.append(at.relation_term(rel)?);
        text.push(" key (");
        for (n, f) in fields.iter().enumerate() {
            if n > 0 {
                text.push(", ");
            }
            text.append(at.decl_range(i, before.local(i, f.span))?);
        }
        text.push(")");
        for (n, p) in carry.iter().enumerate() {
            text.push(if n == 0 { " carry " } else { ", " });
            text.append(at.path(p)?);
        }
        Ok(vec![
            Inverse::Apply { text, of: at.of() },
            Inverse::KeyClause {
                relation: rel,
                text: at.decl_range(i, ranges[k].clone())?,
                of: at.of(),
            },
        ])
    }

    fn default_inverse(
        &self,
        at: Inverting,
        rel: u64,
        ctor: Option<&ast::Ident>,
        field: &ast::Ident,
    ) -> R<Vec<Inverse>> {
        let Inverting { before, .. } = at;
        let path = at.default_path(rel, ctor, field)?;
        let missing = || vec![Diag::new(self.span, "the log does not hold the default")];
        let i = at
            .before_m
            .decl_at(Item::Relation(rel))
            .ok_or_else(missing)?;
        let Some(Decl::Relation(rd)) = before.parsed(i) else {
            return Err(missing().into());
        };
        let Some(value) = rd.clauses.iter().find_map(|c| match c {
            RelationClause::Default {
                ctor: dc,
                field: f,
                value,
            } if f.text == field.text
                && dc.as_ref().map(|x| &x.text) == ctor.as_ref().map(|x| &x.text) =>
            {
                Some(value)
            }
            _ => None,
        }) else {
            return Err(missing().into());
        };
        let mut text = Term::from("add default ");
        text.append(path);
        text.push(" = ");
        match value {
            ast::DefaultValue::Generate(g) => text.push(format!("generate {}", g.text)),
            ast::DefaultValue::Expr(e) => text.append(at.decl_range(i, before.local(i, e.span))?),
        }
        apply(text, at.of())
    }

    fn computed_inverse(&self, at: Inverting, rel: u64, field: &ast::Ident) -> R<Vec<Inverse>> {
        let Inverting { before, .. } = at;
        let missing = || {
            vec![Diag::new(
                self.span,
                "the log does not hold the computed field",
            )]
        };
        let i = at
            .before_m
            .decl_at(Item::Relation(rel))
            .ok_or_else(missing)?;
        let Some(Decl::Relation(rd)) = before.parsed(i) else {
            return Err(missing().into());
        };
        let Some((t, e)) = rd.clauses.iter().find_map(|c| match c {
            RelationClause::Computed(n, t, e) if n.text == field.text => Some((t.span, e.span)),
            _ => None,
        }) else {
            return Err(missing().into());
        };
        let verb = if matches!(at.resolved, Op::DropComputed { .. }) {
            "add"
        } else {
            "redefine"
        };
        let mut text = Term::from(format!("{verb} computed "));
        text.append(at.default_path(rel, None, field)?);
        text.push(": ");
        text.append(at.decl_range(i, before.local(i, t))?);
        text.push(" = ");
        text.append(at.decl_range(i, before.local(i, e))?);
        apply(text, at.of())
    }
}

impl Inverting<'_> {
    /// Returns field `f` of the model after the operator, which the operator adds, as a path
    /// qualified by its module, each part held by its id.
    fn added_field_path(&self, f: u64) -> R<Term> {
        let m = self.after_m;
        let missing = || vec![Diag::new(self.op.span, "the log does not hold the field")];
        let field = m.fields.get(&f).ok_or_else(missing)?;
        let t = m.field_type(f).ok_or_else(missing)?;
        let mut out = self.type_term(t)?;
        if let Some(c) = m.ctors.get(&field.owner)
            && !matches!(m.types[&t].body, TypeBody::Record { .. })
        {
            out.push(".");
            out.push_name(&c.name, Space::Shared, Some(field.owner));
        }
        out.push(".");
        out.push_name(&field.name, Space::Field, Some(f));
        Ok(out)
    }

    /// Returns relation variable `rel` qualified by its module, then constructor `ctor` and field
    /// `field` as the operator writes them.
    fn default_path(&self, rel: u64, ctor: Option<&ast::Ident>, field: &ast::Ident) -> R<Term> {
        let mut out = self.relation_term(rel)?;
        let from = ctor.map_or(field.span.start, |c| c.span.start) as usize;
        out.push(".");
        out.append(self.op_text(from..field.span.end as usize)?);
        Ok(out)
    }

    /// Returns the step that puts the fields of the part holding field `f` back in the order the
    /// model before the operator holds them.
    pub(super) fn reorder(&self, f: u64) -> Inverse {
        let m = self.before_m;
        let owner = m.fields[&f].owner;
        let t = m.field_type(f).expect("a field of a type");
        let (ctor, order) = match m.ctors.get(&owner) {
            Some(c) => {
                let part = (!matches!(m.types[&t].body, TypeBody::Record { .. })).then_some(owner);
                (part, c.fields.clone().unwrap_or_default())
            }
            None => (None, m.types[&t].header.clone().unwrap_or_default()),
        };
        Inverse::Reorder {
            ty: t,
            ctor,
            order,
            of: self.of(),
        }
    }
}
