use super::*;
use crate::canon::canonical_term;

#[cfg(test)]
#[path = "../resolved_tests.rs"]
mod tests;

/// An operator of a logged migration, naming every object it touches by its catalog id, with each
/// operand written in the language in canonical token form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    AddModule {
        module: String,
    },
    AddType {
        sum: u64,
        decl: String,
    },
    AddRelation {
        relation: u64,
        decl: String,
    },
    AddSet {
        set: u64,
        relation: u64,
        name: String,
    },
    AddUnique {
        index: u64,
        relation: u64,
        fields: String,
    },
    AddIndex {
        index: u64,
        relation: u64,
        interval: bool,
        fields: String,
    },
    DefineFn {
        function: u64,
        decl: String,
    },
    DefineAggregate {
        function: u64,
        decl: String,
    },
    AddConstraint {
        constraint: u64,
        body: String,
    },
    AddCheck {
        constraint: u64,
        body: String,
    },
}

/// A clause of a resolved operator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Clause {
    /// A constructor a named set lists, or one a unique or index written with `in` ranges over.
    Member { ctor: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedOp {
    pub op: Resolved,
    /// In clause order.
    pub clauses: Vec<Clause>,
}

/// A function or aggregate of a source, as the catalog records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub id: u64,
    pub module: String,
    pub name: String,
    /// The type parameters, parameters and result type, as written.
    pub signature: String,
    /// Whether every function it calls, transitively, is deterministic, a host function only when
    /// declared so.
    pub deterministic: bool,
    pub code: Code,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Code {
    /// `body` as written, and `canonical` its canonical token form.
    Language {
        body: String,
        canonical: String,
    },
    /// Everything after the `=` of the declaration, as written and in canonical token form.
    Aggregate {
        body: String,
        canonical: String,
    },
    Wasm {
        module: String,
        export: String,
    },
    Host {
        symbol: String,
        version: String,
    },
}

impl Source {
    /// Returns the operators of migration `number` of the log in ordinal order, resolved against the
    /// source after it.
    ///
    /// Fails with the operators that have no resolved form, at their spans, or with the operands whose
    /// names do not resolve. Panics if the log holds no migration `number`.
    pub fn resolved(&self, number: u64) -> Result<Vec<ResolvedOp>, Vec<Diag>> {
        let i = self
            .log
            .iter()
            .position(|l| l.number == number)
            .unwrap_or_else(|| panic!("the log holds migration {number}"));
        let logged = &self.log[i];
        let after = self.log.get(i + 1).map_or(self, |l| &*l.before);
        let unresolvable: Vec<Diag> = logged
            .operators
            .iter()
            .filter(|op| !resolvable(&op.kind))
            .map(|op| Diag::new(op.span, "this operator has no resolved form yet"))
            .collect();
        if !unresolvable.is_empty() {
            return Err(unresolvable);
        }
        let names = logged.names()?;
        let unbuilt = |ds: Vec<FileDiag>| ds.into_iter().map(|d| d.diag).collect::<Vec<_>>();
        let mut r = Resolver {
            logged,
            schema: &after.schema().map_err(unbuilt)?,
            before: &logged.before.schema().map_err(unbuilt)?,
            names: &names,
            indexes: BTreeMap::new(),
        };
        logged.operators.iter().map(|op| r.operator(op)).collect()
    }

    /// Returns every function and aggregate of the user modules, in declaration order.
    pub fn functions(&self) -> Result<Vec<Function>, Vec<FileDiag>> {
        let schema = self.schema()?;
        let names = self.names()?;
        let deterministic = deterministic_functions(&schema);
        let mut out = Vec::new();
        for (file, f) in self.files()? {
            let Some(module) = schema.module_named(&f.module.text) else {
                continue;
            };
            let text = self.file_text(file);
            let names = &names[file.0 as usize];
            let term = |span: Span| {
                (
                    text[span.range()].to_string(),
                    canonical_term(&text, span, names),
                )
            };
            let values = &schema.module(module).values;
            for d in &f.decls {
                let (id, name, result, det, code) = match d {
                    Decl::Fn(fd) => {
                        let Some(&ValueDef::Function(fid)) = values.get(&fd.name.text) else {
                            continue;
                        };
                        let code = match &fd.body {
                            ast::FnBody::Expr(e) => {
                                let (body, canonical) = term(e.span);
                                Code::Language { body, canonical }
                            }
                            ast::FnBody::Wasm { module, export } => Code::Wasm {
                                module: module.clone(),
                                export: export.clone(),
                            },
                            ast::FnBody::Host { name, version, .. } => Code::Host {
                                symbol: name.clone(),
                                version: version.clone(),
                            },
                        };
                        (
                            schema.ids.functions.get(&fid),
                            &fd.name,
                            fd.result.span,
                            deterministic[fid.0 as usize],
                            code,
                        )
                    }
                    Decl::Aggregate(a) => {
                        let Some(&ValueDef::Aggregate(i)) = values.get(&a.name.text) else {
                            continue;
                        };
                        let (body, canonical) =
                            term(after_eq(&text, a.result.span.end, a.span.end));
                        let id = schema.ids.aggregates.get(&(i as usize));
                        (
                            id,
                            &a.name,
                            a.result.span,
                            crate::resolve::aggregate_refs(&schema, i as usize)
                                .iter()
                                .all(|g| deterministic[g.0 as usize]),
                            Code::Aggregate { body, canonical },
                        )
                    }
                    _ => continue,
                };
                let Some(&id) = id else { continue };
                out.push(Function {
                    id,
                    module: f.module.text.clone(),
                    name: name.text.clone(),
                    signature: text[name.span.end as usize..result.end as usize]
                        .trim()
                        .to_string(),
                    deterministic: det,
                    code,
                });
            }
        }
        Ok(out)
    }
}

/// Returns, for each function of `schema`, whether every function it calls, transitively, is
/// deterministic.
fn deterministic_functions(schema: &Schema) -> Vec<bool> {
    let calls: Vec<BTreeSet<FnId>> = (0..schema.functions.len() as u32)
        .map(|i| crate::resolve::function_refs(schema, FnId(i)))
        .collect();
    let mut det: Vec<bool> = schema
        .functions
        .iter()
        .map(|f| {
            !matches!(
                f.body,
                ast::FnBody::Host {
                    deterministic: false,
                    ..
                }
            )
        })
        .collect();
    let mut changed = true;
    while changed {
        changed = false;
        for (i, gs) in calls.iter().enumerate() {
            if det[i] && gs.iter().any(|g| !det[g.0 as usize]) {
                det[i] = false;
                changed = true;
            }
        }
    }
    det
}

/// Returns the span from the first token after the first `=` at or after `from` to `end`.
fn after_eq(text: &str, from: u32, end: u32) -> Span {
    let toks = tokens(&text[from as usize..end as usize]);
    let at = toks
        .iter()
        .position(|(t, _)| *t == Tok::Eq)
        .and_then(|i| toks.get(i + 1))
        .map_or(end as usize, |(_, r)| from as usize + r.start);
    Span::new(at, end as usize)
}

fn resolvable(op: &OperatorKind) -> bool {
    matches!(
        op,
        OperatorKind::AddModule(_)
            | OperatorKind::AddType(_)
            | OperatorKind::AddRelation(_)
            | OperatorKind::AddSet(_)
            | OperatorKind::AddUnique(..)
            | OperatorKind::AddIndex { .. }
            | OperatorKind::DefineFn(_)
            | OperatorKind::DefineAggregate(_)
            | OperatorKind::AddConstraint(_)
            | OperatorKind::AddCheck(_)
    )
}

struct Resolver<'a> {
    logged: &'a Logged,
    /// The schema after the migration.
    schema: &'a Schema,
    /// The schema before the migration.
    before: &'a Schema,
    names: &'a Names,
    /// The position among the relation's uniques and indexes the next one added takes.
    indexes: BTreeMap<RelId, usize>,
}

impl Resolver<'_> {
    fn operator(&mut self, op: &Operator) -> Result<ResolvedOp, Vec<Diag>> {
        let gone = || {
            vec![Diag::new(
                op.span,
                "an object this operator creates or names is gone by the migration's end",
            )]
        };
        let term = |span: Span| canonical_term(&self.logged.text, span, self.names);
        let own = |name: &ast::Ident| QName {
            module: None,
            name: name.clone(),
        };
        let mut clauses = Vec::new();
        let resolved = match &op.kind {
            OperatorKind::AddModule(m) => Resolved::AddModule {
                module: m.text.clone(),
            },
            OperatorKind::AddType(t) => Resolved::AddType {
                sum: self.type_id(&own(&t.name)).ok_or_else(gone)?,
                decl: term(t.span),
            },
            OperatorKind::AddRelation(r) => {
                let rel = self.relation(&r.name).ok_or_else(gone)?;
                let inline = r
                    .clauses
                    .iter()
                    .filter(|c| {
                        matches!(c, RelationClause::Unique(..) | RelationClause::Index { .. })
                    })
                    .count();
                self.indexes.insert(rel, inline);
                Resolved::AddRelation {
                    relation: self.schema.ids.relations[&rel],
                    decl: term(r.span),
                }
            }
            OperatorKind::AddSet(s) => {
                let rel = self.relation(&own(&s.relation)).ok_or_else(gone)?;
                let set = *self
                    .schema
                    .ids
                    .sets
                    .get(&(rel, s.name.text.clone()))
                    .ok_or_else(gone)?;
                for m in &s.members {
                    clauses.push(Clause::Member {
                        ctor: self.ctor(rel, &m.text).ok_or_else(gone)?,
                    });
                }
                Resolved::AddSet {
                    set,
                    relation: self.schema.ids.relations[&rel],
                    name: s.name.text.clone(),
                }
            }
            OperatorKind::AddUnique(fields, on) | OperatorKind::AddIndex { fields, on, .. } => {
                let rel = self.relation(&on.relation).ok_or_else(gone)?;
                let index = self.next_index(rel, &on.relation).ok_or_else(gone)?;
                for c in on.sel.iter().flat_map(sel_idents) {
                    clauses.push(Clause::Member {
                        ctor: self.ctor(rel, &c.text).ok_or_else(gone)?,
                    });
                }
                let relation = self.schema.ids.relations[&rel];
                let fields = match (fields.first(), fields.last()) {
                    (Some(a), Some(b)) => term(a.span.to(b.span)),
                    _ => String::new(),
                };
                match &op.kind {
                    OperatorKind::AddIndex { interval, .. } => Resolved::AddIndex {
                        index,
                        relation,
                        interval: *interval,
                        fields,
                    },
                    _ => Resolved::AddUnique {
                        index,
                        relation,
                        fields,
                    },
                }
            }
            OperatorKind::DefineFn(f) => Resolved::DefineFn {
                function: self.function(&f.name).ok_or_else(gone)?,
                decl: term(f.span),
            },
            OperatorKind::DefineAggregate(a) => Resolved::DefineAggregate {
                function: self.function(&a.name).ok_or_else(gone)?,
                decl: term(a.span),
            },
            OperatorKind::AddConstraint(c) => Resolved::AddConstraint {
                constraint: self.constraint(&c.name).ok_or_else(gone)?,
                body: term(c.span),
            },
            OperatorKind::AddCheck(c) => Resolved::AddCheck {
                constraint: self.constraint(&c.name).ok_or_else(gone)?,
                body: term(c.span),
            },
            _ => return Err(gone()),
        };
        Ok(ResolvedOp {
            op: resolved,
            clauses,
        })
    }

    fn module_of(&self, q: &QName) -> String {
        q.module
            .as_ref()
            .map_or(self.logged.module.clone(), |m| m.text.clone())
    }

    fn value(&self, schema: &Schema, q: &QName) -> Option<ValueDef> {
        let m = schema.module_named(&self.module_of(q))?;
        schema.module(m).values.get(&q.name.text).copied()
    }

    fn type_id(&self, q: &QName) -> Option<u64> {
        let m = self.schema.module_named(&self.module_of(q))?;
        let def = self.schema.module(m).types.get(&q.name.text)?;
        self.schema.ids.types.get(def).copied()
    }

    fn relation(&self, q: &QName) -> Option<RelId> {
        match self.value(self.schema, q)? {
            ValueDef::Relation(r) => Some(r),
            _ => None,
        }
    }

    fn ctor(&self, rel: RelId, name: &str) -> Option<u64> {
        let def = self.schema.relation(rel).elem_def;
        let c = self.schema.ty(def).ctor_index(name)?;
        self.schema.ids.ctors.get(&(def, c)).copied()
    }

    /// Returns the id of the unique or index an operator appends to `rel`, `q` naming it.
    fn next_index(&mut self, rel: RelId, q: &QName) -> Option<u64> {
        let at = match self.indexes.get(&rel) {
            Some(&n) => n,
            None => match self.value(self.before, q)? {
                ValueDef::Relation(r) => self.before.relation(r).indexes.len(),
                _ => return None,
            },
        };
        self.indexes.insert(rel, at + 1);
        self.schema.ids.indexes.get(&(rel, at)).copied()
    }

    fn function(&self, name: &ast::Ident) -> Option<u64> {
        let q = QName {
            module: None,
            name: name.clone(),
        };
        match self.value(self.schema, &q)? {
            ValueDef::Function(f) => self.schema.ids.functions.get(&f).copied(),
            ValueDef::Aggregate(i) => self.schema.ids.aggregates.get(&(i as usize)).copied(),
            _ => None,
        }
    }

    fn constraint(&self, name: &ast::Ident) -> Option<u64> {
        let q = QName {
            module: None,
            name: name.clone(),
        };
        match self.value(self.schema, &q)? {
            ValueDef::Constraint(i) => self.schema.ids.constraints.get(&(i as usize)).copied(),
            ValueDef::Check(i) => self.schema.ids.checks.get(&(i as usize)).copied(),
            _ => None,
        }
    }
}
