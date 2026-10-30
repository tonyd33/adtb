use super::*;
use crate::canon::canonical_term;

#[path = "derived.rs"]
mod derived;
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
    DropModule {
        module: String,
    },
    DropType {
        sum: u64,
    },
    DropRelation {
        relation: u64,
    },
    DropSet {
        set: u64,
    },
    AddConstructor {
        ctor: u64,
        sum: u64,
        decl: String,
        place: Place,
    },
    RemoveConstructor {
        ctor: u64,
    },
    Enable {
        target: Target,
    },
    Disable {
        target: Target,
    },
    Deprecate {
        target: Target,
    },
    RenameConstructor {
        ctor: u64,
        name: String,
    },
    RenameType {
        sum: u64,
        name: String,
    },
    RenameRelation {
        relation: u64,
        name: String,
    },
    RenameSet {
        set: u64,
        name: String,
    },
    RenameField {
        field: u64,
        name: String,
    },
    Split {
        ctor: u64,
        decl: String,
        routing: String,
    },
    Merge {
        ctor: u64,
        decl: String,
        routing: Option<String>,
    },
    Nest {
        ctor: u64,
        decl: String,
    },
    Unnest {
        ctor: u64,
    },
    AddField {
        field: u64,
        decl: String,
        fill: String,
    },
    DropField {
        field: u64,
    },
    AlterField {
        field: u64,
        action: String,
    },
    MoveField {
        field: u64,
        header: bool,
        fill: Option<String>,
    },
    DropUnique {
        index: u64,
    },
    DropIndex {
        index: u64,
    },
    Rekey {
        relation: u64,
        key: u64,
        fields: String,
    },
    RedefineFn {
        function: u64,
        decl: String,
    },
    DropFn {
        function: u64,
    },
    DropAggregate {
        function: u64,
    },
    DropConstraint {
        constraint: u64,
    },
    DropCheck {
        constraint: u64,
    },
    Revert {
        migration: u64,
    },
    /// One for each field the default fills, the first of them named.
    AddDefault {
        default: u64,
        relation: u64,
        field: u64,
        value: String,
    },
    DropDefault {
        default: u64,
    },
    Store {
        view: u64,
        function: u64,
        decl: String,
    },
    Unstore {
        view: u64,
    },
    AddComputed {
        computed: u64,
        relation: u64,
        name: String,
        decl: String,
    },
    RedefineComputed {
        computed: u64,
        decl: String,
    },
    DropComputed {
        computed: u64,
    },
    /// The first privilege of the statement; a `Privilege` clause holds each later one.
    Grant {
        privilege: String,
        target: String,
        grantee: String,
        grant_option: bool,
    },
    /// The first privilege of the statement; a `Privilege` clause holds each later one.
    Revoke {
        privilege: String,
        target: String,
        grantee: String,
    },
    /// `package` is the package's contents.
    Install {
        module: String,
        version: String,
        package: String,
    },
    Uninstall {
        module: String,
    },
    /// `data` is the release's data.
    AdoptTzdb {
        release: String,
        data: String,
    },
    Script {
        write: String,
    },
}

/// The constructor relation, or the constructor of a type, a write state operator names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    InRelation { relation: u64, ctor: u64 },
    OfType { ctor: u64 },
}

/// Where an added constructor goes in its type's declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    Last,
    Before { ctor: u64 },
    After { ctor: u64 },
}

/// A clause of a resolved operator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Clause {
    /// A constructor a named set lists, a merge or nest replaces, a moved field goes to, or a
    /// unique or index written with `in` ranges over.
    Member { ctor: u64 },
    /// A named set an added constructor joins.
    Joins { set: u64 },
    /// An `arm` clause: what its arm ends the match of, and the arm.
    Arm { target: ArmTarget, arm: String },
    /// A `unique` or `index` clause of a split: the index it adds, its kind and its fields.
    Replacement {
        index: u64,
        kind: IndexKind,
        fields: String,
    },
    /// A field a rekey carries.
    Carry { field: u64 },
    /// A field a revert fills, with its fill.
    Fill { field: u64, fill: String },
    /// A privilege of a grant or revoke after its first.
    Privilege { privilege: String, target: String },
}

/// The schema object an `arm` clause's arm belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArmTarget {
    OfFn { function: u64 },
    OfConstraint { constraint: u64 },
    OfComputed { computed: u64 },
}

/// The kind of an index a split's replacement clause adds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexKind {
    Unique,
    Plain,
    Interval,
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
    /// Fail with the operators whose data the source lacks, at their spans, or with the operands
    /// whose names do not resolve. Panics if the log holds no migration `number`.
    pub fn resolved(&self, number: u64) -> Result<Vec<ResolvedOp>, Vec<Diag>> {
        let i = self
            .log
            .iter()
            .position(|l| l.number == number)
            .unwrap_or_else(|| panic!("the log holds migration {number}"));
        let logged = &self.log[i];
        let after = self.log.get(i + 1).map_or(self, |l| &*l.before);
        let unavailable: Vec<Diag> = logged
            .operators
            .iter()
            .filter_map(|op| match &op.kind {
                OperatorKind::AdoptTzdb { from, .. } if !self.packages.contains_key(from) => {
                    Some(Diag::new(
                        op.span,
                        format!("the release data `{from}` is not available"),
                    ))
                }
                _ => None,
            })
            .collect();
        if !unavailable.is_empty() {
            return Err(unavailable);
        }
        let names = logged.names()?;
        let unbuilt = |ds: Vec<FileDiag>| ds.into_iter().map(|d| d.diag).collect::<Vec<_>>();
        let mut r = Resolver {
            logged,
            schema: &after.schema().map_err(unbuilt)?,
            before: &logged.before.schema().map_err(unbuilt)?,
            names: &names,
            indexes: BTreeMap::new(),
            at: 0,
        };
        (0..logged.operators.len())
            .map(|k| {
                r.at = k;
                r.operator(&logged.operators[k])
            })
            .collect()
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

struct Resolver<'a> {
    logged: &'a Logged,
    /// The schema after the migration.
    schema: &'a Schema,
    /// The schema before the migration.
    before: &'a Schema,
    names: &'a Names,
    /// The position among the relation's uniques and indexes the next one added takes.
    indexes: BTreeMap<RelId, usize>,
    /// The ordinal of the operator being resolved.
    at: usize,
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
                let index = self.next_index(rel).ok_or_else(gone)?;
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
            kind => self.changing(kind, &mut clauses).ok_or_else(gone)?,
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
        let principal = self.schema.principal;
        let def = match self.schema.module(m).types.get(&q.name.text) {
            Some(def) => *def,
            None if q.name.text == "Principal" && self.schema.ty(principal).module == m => {
                principal
            }
            None => return None,
        };
        self.schema.ids.types.get(&def).copied()
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

    /// Returns the id of the unique or index an operator appends to `rel`.
    fn next_index(&mut self, rel: RelId) -> Option<u64> {
        let at = match self.indexes.get(&rel) {
            Some(&n) => n,
            None => {
                let id = self.schema.ids.relations[&rel];
                let r = self.before.ids.relation(id)?;
                self.before.relation(r).indexes.len()
            }
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

impl Resolver<'_> {
    /// Returns the resolved form of an operator that drops, renames or reshapes objects, its
    /// clauses appended to `clauses`; `None` when an object it names is in neither schema.
    fn changing(&self, kind: &OperatorKind, clauses: &mut Vec<Clause>) -> Option<Resolved> {
        let text = &self.logged.text;
        let term = |span: Span| canonical_term(text, span, self.names);
        let (before, after) = (self.before, self.schema);
        let own = |name: &ast::Ident| QName {
            module: None,
            name: name.clone(),
        };
        Some(match kind {
            OperatorKind::DropModule(m) => Resolved::DropModule {
                module: m.text.clone(),
            },
            OperatorKind::DropType(t) => Resolved::DropType {
                sum: self.type_in(before, t)?,
            },
            OperatorKind::RenameType(t, to) => Resolved::RenameType {
                sum: self
                    .type_in(before, t)
                    .or_else(|| self.type_in(after, &renamed(t, to)))?,
                name: to.text.clone(),
            },
            OperatorKind::DropRelation(r) => Resolved::DropRelation {
                relation: before.ids.relations[&self.relation_in(before, r)?],
            },
            OperatorKind::RenameRelation(r, to) => Resolved::RenameRelation {
                relation: match self.relation_in(before, r) {
                    Some(rel) => before.ids.relations[&rel],
                    None => after.ids.relations[&self.relation_in(after, &renamed(r, to))?],
                },
                name: to.text.clone(),
            },
            OperatorKind::DropSet(set, r) => Resolved::DropSet {
                set: self.set_in(before, r, &set.text)?,
            },
            OperatorKind::RenameSet(set, r, to) => Resolved::RenameSet {
                set: self
                    .set_in(before, r, &set.text)
                    .or_else(|| self.set_in(after, r, &to.text))?,
                name: to.text.clone(),
            },
            OperatorKind::DropUnique(fields, on) => Resolved::DropUnique {
                index: self.index_in(before, fields, on, true)?,
            },
            OperatorKind::DropIndex { fields, on, .. } => Resolved::DropIndex {
                index: self.index_in(before, fields, on, false)?,
            },
            OperatorKind::Rekey {
                relation,
                key,
                carry,
            } => {
                for p in carry {
                    clauses.push(Clause::Carry {
                        field: self
                            .field_in(before, p, &p.field.text)
                            .or_else(|| self.field_in(after, p, &p.field.text))?,
                    });
                }
                let rel = self.relation_in(after, relation)?;
                Resolved::Rekey {
                    relation: after.ids.relations[&rel],
                    key: *after.ids.keys.get(&rel)?,
                    fields: term(key.first()?.span.to(key.last()?.span)),
                }
            }
            OperatorKind::RedefineFn(f) => Resolved::RedefineFn {
                function: self.function_in(after, &own(&f.name))?,
                decl: term(f.span),
            },
            OperatorKind::DropFn(f) => Resolved::DropFn {
                function: self.function_in(before, f)?,
            },
            OperatorKind::DropAggregate(f) => Resolved::DropAggregate {
                function: self.function_in(before, f)?,
            },
            OperatorKind::DropConstraint(c) => Resolved::DropConstraint {
                constraint: self.constraint_in(before, c)?,
            },
            OperatorKind::DropCheck(c) => Resolved::DropCheck {
                constraint: self.constraint_in(before, c)?,
            },
            OperatorKind::Revert { migration, fills } => {
                for (p, f) in fills {
                    clauses.push(Clause::Fill {
                        field: self
                            .field_in(after, p, &p.field.text)
                            .or_else(|| self.field_in(before, p, &p.field.text))?,
                        fill: canonical_term(&self.logged.text, fill_span(f)?, self.names),
                    });
                }
                Resolved::Revert {
                    migration: migration.parse().ok()?,
                }
            }
            kind => return self.reshaping(kind, clauses),
        })
    }

    /// Returns the resolved form of an operator that adds, removes or reshapes constructors or
    /// fields, its clauses appended to `clauses`; `None` when an object it names is in neither
    /// schema or it is no such operator.
    fn reshaping(&self, kind: &OperatorKind, clauses: &mut Vec<Clause>) -> Option<Resolved> {
        let text = &self.logged.text;
        let term = |span: Span| canonical_term(text, span, self.names);
        let def = |name: &ast::Ident| term(ctor_def_span(text, name));
        let (before, after) = (self.before, self.schema);
        Some(match kind {
            OperatorKind::AddConstructor {
                path,
                place,
                joins,
                arms,
                ..
            } => {
                let ctor = self.type_ctor(after, &path.ty, &path.ctor.text)?;
                let place = match place {
                    None => Place::Last,
                    Some(CtorPlace::Before(c)) => Place::Before {
                        ctor: self.type_ctor(after, &path.ty, &c.text)?,
                    },
                    Some(CtorPlace::After(c)) => Place::After {
                        ctor: self.type_ctor(after, &path.ty, &c.text)?,
                    },
                };
                for j in joins {
                    for s in j.sel.iter().flat_map(sel_idents) {
                        clauses.push(Clause::Joins {
                            set: self.set_in(after, &j.relation, &s.text)?,
                        });
                    }
                }
                for a in arms {
                    clauses.push(self.arm_clause(a)?);
                }
                Resolved::AddConstructor {
                    ctor,
                    sum: self.type_in(after, &path.ty)?,
                    decl: def(&path.ctor),
                    place,
                }
            }
            OperatorKind::RemoveConstructor(path) => Resolved::RemoveConstructor {
                ctor: self.type_ctor(before, &path.ty, &path.ctor.text)?,
            },
            OperatorKind::RenameConstructor(path, to) => Resolved::RenameConstructor {
                ctor: self
                    .type_ctor(before, &path.ty, &path.ctor.text)
                    .or_else(|| self.type_ctor(after, &path.ty, &to.text))?,
                name: to.text.clone(),
            },
            OperatorKind::Enable(t) => Resolved::Enable {
                target: self.target(t)?,
            },
            OperatorKind::Disable(t) => Resolved::Disable {
                target: self.target(t)?,
            },
            OperatorKind::Deprecate(t) => Resolved::Deprecate {
                target: self.target(t)?,
            },
            OperatorKind::AddField {
                path, decl, fill, ..
            } => Resolved::AddField {
                field: self.field_in(after, path, &self.renamed_later(path))?,
                decl: term(*decl),
                fill: term(fill_span(fill)?),
            },
            OperatorKind::AlterField { path, span, .. } => Resolved::AlterField {
                field: self.field_in(before, path, &path.field.text)?,
                action: term(*span),
            },
            OperatorKind::DropField(path) => Resolved::DropField {
                field: self.field_in(before, path, &path.field.text)?,
            },
            OperatorKind::RenameField(path, to) => Resolved::RenameField {
                field: self
                    .field_in(before, path, &path.field.text)
                    .or_else(|| self.field_in(after, path, &to.text))?,
                name: to.text.clone(),
            },
            OperatorKind::MoveField {
                path,
                to,
                sel,
                fill,
            } => {
                let header = sel.is_none();
                let field = if header {
                    let into = ast::FieldPath {
                        ty: to.clone(),
                        ctor: None,
                        field: path.field.clone(),
                    };
                    self.field_in(after, &into, &path.field.text)?
                } else {
                    let first = sel.iter().flat_map(sel_idents).next()?;
                    for c in sel.iter().flat_map(sel_idents) {
                        clauses.push(Clause::Member {
                            ctor: self.type_ctor(after, to, &c.text)?,
                        });
                    }
                    let into = ast::FieldPath {
                        ty: to.clone(),
                        ctor: Some(first.clone()),
                        field: path.field.clone(),
                    };
                    self.field_in(after, &into, &path.field.text)?
                };
                Resolved::MoveField {
                    field,
                    header,
                    fill: match fill {
                        Some(f) => Some(term(fill_span(f)?)),
                        None => None,
                    },
                }
            }
            kind => return self.restructuring(kind, clauses),
        })
    }

    /// Returns the resolved form of an operator that splits, merges, nests or unnests
    /// constructors, its clauses appended to `clauses`; `None` when an object it names is in
    /// neither schema or it is no such operator.
    fn restructuring(&self, kind: &OperatorKind, clauses: &mut Vec<Clause>) -> Option<Resolved> {
        let text = &self.logged.text;
        let term = |span: Span| canonical_term(text, span, self.names);
        let def = |name: &ast::Ident| term(ctor_def_span(text, name));
        let (before, after) = (self.before, self.schema);
        Some(match kind {
            OperatorKind::SplitConstructor {
                path,
                into,
                arms,
                clauses: split,
            } => {
                clauses.extend(self.split_clauses(&path.ty, split)?);
                Resolved::Split {
                    ctor: self.type_ctor(before, &path.ty, &path.ctor.text)?,
                    decl: term(
                        ctor_def_span(text, &into.first()?.name)
                            .to(ctor_def_span(text, &into.last()?.name)),
                    ),
                    routing: arms_term(arms).map_or_else(String::new, term),
                }
            }
            OperatorKind::MergeConstructor {
                ty,
                sel,
                into,
                arms,
                clauses: given,
            } => {
                clauses.extend(self.replaced(ty, sel)?);
                for a in given {
                    clauses.push(self.arm_clause(a)?);
                }
                Resolved::Merge {
                    ctor: self.type_ctor(after, ty, &into.name.text)?,
                    decl: def(&into.name),
                    routing: arms.as_deref().and_then(arms_term).map(term),
                }
            }
            OperatorKind::NestConstructor {
                ty,
                sel,
                into,
                clauses: given,
            } => {
                clauses.extend(self.replaced(ty, sel)?);
                for a in given {
                    clauses.push(self.arm_clause(a)?);
                }
                Resolved::Nest {
                    ctor: self.type_ctor(after, ty, &into.name.text)?,
                    decl: def(&into.name),
                }
            }
            OperatorKind::UnnestConstructor(path, given) => {
                for a in given {
                    clauses.push(self.arm_clause(a)?);
                }
                Resolved::Unnest {
                    ctor: self.type_ctor(before, &path.ty, &path.ctor.text)?,
                }
            }
            kind => return self.derived(kind, clauses),
        })
    }

    /// Returns the clause of `arm` clause `a`.
    fn arm_clause(&self, a: &ast::ArmClause) -> Option<Clause> {
        let after = self.schema;
        let target = match &a.field {
            Some(f) => {
                let rel = self.relation_in(after, &a.target)?;
                let at = after
                    .relation(rel)
                    .computed
                    .iter()
                    .position(|c| c.name == f.text)?;
                ArmTarget::OfComputed {
                    computed: *after.ids.computed.get(&(rel, at))?,
                }
            }
            None => match self.function_in(after, &a.target) {
                Some(function) => ArmTarget::OfFn { function },
                None => ArmTarget::OfConstraint {
                    constraint: self.constraint_in(after, &a.target)?,
                },
            },
        };
        Some(Clause::Arm {
            target,
            arm: canonical_term(
                &self.logged.text,
                a.pattern.span.to(a.body.span),
                self.names,
            ),
        })
    }

    /// Returns a `Member` clause for each constructor of `ty` that `sel` names, before the
    /// operator.
    fn replaced(&self, ty: &QName, sel: &CtorSel) -> Option<Vec<Clause>> {
        sel_idents(sel)
            .iter()
            .map(|c| {
                Some(Clause::Member {
                    ctor: self.type_ctor(self.before, ty, &c.text)?,
                })
            })
            .collect()
    }

    /// Returns the clauses of a split of a constructor of `ty`, in order: each `unique` or
    /// `index` clause takes, in order, the last indexes of `ty`'s relation the migration gives new
    /// ids.
    fn split_clauses(&self, ty: &QName, given: &[ast::SplitClause]) -> Option<Vec<Clause>> {
        let term = |span: Span| canonical_term(&self.logged.text, span, self.names);
        let added = given
            .iter()
            .filter(|c| !matches!(c, ast::SplitClause::Arm(_)))
            .count();
        let mut fresh = Vec::new();
        if added > 0 {
            let def = self.type_def(self.schema, ty)?;
            let old: BTreeSet<u64> = self.before.ids.indexes.values().copied().collect();
            let (&rel, _) = self
                .schema
                .ids
                .relations
                .iter()
                .find(|&(&r, _)| self.schema.relation(r).elem_def == def)?;
            fresh = self
                .schema
                .ids
                .indexes
                .range((rel, 0)..=(rel, usize::MAX))
                .map(|(_, &id)| id)
                .filter(|id| !old.contains(id))
                .collect();
            fresh.drain(..fresh.len().checked_sub(added)?);
        }
        let mut fresh = fresh.into_iter();
        let mut out = Vec::new();
        for c in given {
            let (kind, fields) = match c {
                ast::SplitClause::Arm(a) => {
                    out.push(self.arm_clause(a)?);
                    continue;
                }
                ast::SplitClause::Unique(fs, _) => (IndexKind::Unique, fs),
                ast::SplitClause::Index {
                    interval, fields, ..
                } => (
                    if *interval {
                        IndexKind::Interval
                    } else {
                        IndexKind::Plain
                    },
                    fields,
                ),
            };
            out.push(Clause::Replacement {
                index: fresh.next()?,
                kind,
                fields: term(fields.first()?.span.to(fields.last()?.span)),
            });
        }
        Some(out)
    }

    fn type_in(&self, schema: &Schema, q: &QName) -> Option<u64> {
        let m = schema.module_named(&self.module_of(q))?;
        let def = schema.module(m).types.get(&q.name.text)?;
        schema.ids.types.get(def).copied()
    }

    fn relation_in(&self, schema: &Schema, q: &QName) -> Option<RelId> {
        let m = schema.module_named(&self.module_of(q))?;
        match schema.module(m).values.get(&q.name.text)? {
            ValueDef::Relation(r) => Some(*r),
            _ => None,
        }
    }

    fn set_in(&self, schema: &Schema, r: &QName, set: &str) -> Option<u64> {
        let rel = self.relation_in(schema, r)?;
        schema.ids.sets.get(&(rel, set.to_string())).copied()
    }

    fn type_def(&self, schema: &Schema, q: &QName) -> Option<TypeId> {
        let m = schema.module_named(&self.module_of(q))?;
        schema.module(m).types.get(&q.name.text).copied()
    }

    fn type_ctor(&self, schema: &Schema, ty: &QName, ctor: &str) -> Option<u64> {
        let def = self.type_def(schema, ty)?;
        let c = schema.ty(def).ctor_index(ctor)?;
        schema.ids.ctors.get(&(def, c)).copied()
    }

    /// Returns the name the field at `path` has after the renames of the operators after this one.
    fn renamed_later(&self, path: &ast::FieldPath) -> String {
        let mut name = path.field.text.clone();
        for op in &self.logged.operators[self.at + 1..] {
            if let OperatorKind::RenameField(p, to) = &op.kind
                && self.module_of(&p.ty) == self.module_of(&path.ty)
                && p.ty.name.text == path.ty.name.text
                && p.ctor.as_ref().map(|c| &c.text) == path.ctor.as_ref().map(|c| &c.text)
                && p.field.text == name
            {
                name.clone_from(&to.text);
            }
        }
        name
    }

    fn field_in(&self, schema: &Schema, path: &ast::FieldPath, name: &str) -> Option<u64> {
        let def = self.type_def(schema, &path.ty)?;
        let ctor = match &path.ctor {
            Some(c) => Some(schema.ty(def).ctor_index(&c.text)?),
            None => None,
        };
        let id = schema.ids.fields.get(&(def, ctor, name.to_string()));
        id.or_else(|| match ctor {
            None => schema.ids.fields.get(&(def, Some(0), name.to_string())),
            Some(_) => None,
        })
        .copied()
    }

    fn target(&self, t: &CtorTarget) -> Option<Target> {
        Some(match t {
            CtorTarget::Relation(on) => {
                let c = on.sel.iter().flat_map(sel_idents).next()?;
                let found = [self.schema, self.before].into_iter().find_map(|s| {
                    let rel = self.relation_in(s, &on.relation)?;
                    let def = s.relation(rel).elem_def;
                    let k = s.ty(def).ctor_index(&c.text)?;
                    Some((s.ids.relations[&rel], *s.ids.ctors.get(&(def, k))?))
                })?;
                Target::InRelation {
                    relation: found.0,
                    ctor: found.1,
                }
            }
            CtorTarget::Type(path) => Target::OfType {
                ctor: self
                    .type_ctor(self.schema, &path.ty, &path.ctor.text)
                    .or_else(|| self.type_ctor(self.before, &path.ty, &path.ctor.text))?,
            },
        })
    }

    fn index_in(
        &self,
        schema: &Schema,
        fields: &[Expr],
        on: &ast::RelationRef,
        unique: bool,
    ) -> Option<u64> {
        let rel = self.relation_in(schema, &on.relation)?;
        let def = schema.relation(rel).elem_def;
        let names = |es: &[Expr]| -> Option<Vec<String>> {
            es.iter()
                .map(|e| match &e.kind {
                    ast::ExprKind::Name(q) if q.module.is_none() => Some(q.name.text.clone()),
                    _ => None,
                })
                .collect()
        };
        let wanted = names(fields)?;
        let scope: Option<Vec<u32>> = match &on.sel {
            None => None,
            Some(sel) => Some(
                sel_idents(sel)
                    .iter()
                    .map(|c| schema.ty(def).ctor_index(&c.text))
                    .collect::<Option<_>>()?,
            ),
        };
        let at = schema.relation(rel).indexes.iter().position(|d| {
            d.unique == unique
                && names(&d.fields).as_ref() == Some(&wanted)
                && d.scope.as_ref().map(|s| s.to_vec()) == scope
        })?;
        schema.ids.indexes.get(&(rel, at)).copied()
    }

    fn function_in(&self, schema: &Schema, q: &QName) -> Option<u64> {
        let m = schema.module_named(&self.module_of(q))?;
        match schema.module(m).values.get(&q.name.text)? {
            ValueDef::Function(f) => schema.ids.functions.get(f).copied(),
            ValueDef::Aggregate(i) => schema.ids.aggregates.get(&(*i as usize)).copied(),
            _ => None,
        }
    }

    fn constraint_in(&self, schema: &Schema, q: &QName) -> Option<u64> {
        let m = schema.module_named(&self.module_of(q))?;
        match schema.module(m).values.get(&q.name.text)? {
            ValueDef::Constraint(i) => schema.ids.constraints.get(&(*i as usize)).copied(),
            ValueDef::Check(i) => schema.ids.checks.get(&(*i as usize)).copied(),
            _ => None,
        }
    }
}

/// Returns `q` with its name `to`.
fn renamed(q: &QName, to: &ast::Ident) -> QName {
    QName {
        module: q.module.clone(),
        name: to.clone(),
    }
}

/// Returns the span from the first arm's pattern to the last arm's body.
fn arms_term(arms: &[ast::Arm]) -> Option<Span> {
    let first = arms.first()?.patterns.first()?.span;
    Some(first.to(arms.last()?.body.span))
}

/// Returns the span of the constructor definition named at `name`, through its record's braces.
fn ctor_def_span(text: &str, name: &ast::Ident) -> Span {
    let end = braces_at(text, name.span.end as usize).map_or(name.span.end as usize, |b| b.end);
    Span::new(name.span.start as usize, end)
}

fn fill_span(fill: &Fill) -> Option<Span> {
    match fill {
        Fill::Expr(e) => Some(e.span),
        Fill::Match(arms) => arms_term(arms),
    }
}
