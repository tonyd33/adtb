use super::*;
use crate::ast::{self, CtorPlace, CtorSel, CtorTarget, Expr, OperatorKind, QName};
use crate::lex::{Tok, lex};
use crate::schema::Stage;
use crate::span::{Diag, DiagKind, FileDiag, Span};

/// The kind of a declaration, as `add` and `drop` operators name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Kind {
    Type,
    Relation,
    Set,
    Fn,
    Aggregate,
    Constraint,
    Check,
    Store,
}

impl Kind {
    pub(super) fn word(self) -> &'static str {
        match self {
            Kind::Type => "type",
            Kind::Relation => "relation",
            Kind::Set => "set",
            Kind::Fn => "fn",
            Kind::Aggregate => "aggregate",
            Kind::Constraint => "constraint",
            Kind::Check => "check",
            Kind::Store => "store",
        }
    }

    /// Returns the kind whose keyword is `word`.
    pub(super) fn named(word: &str) -> Option<Kind> {
        [
            Kind::Type,
            Kind::Relation,
            Kind::Set,
            Kind::Fn,
            Kind::Aggregate,
            Kind::Constraint,
            Kind::Check,
            Kind::Store,
        ]
        .into_iter()
        .find(|k| k.word() == word)
    }

    fn of(item: Item) -> Kind {
        match item {
            Item::Type(_) => Kind::Type,
            Item::Relation(_) => Kind::Relation,
            Item::Set(_) => Kind::Set,
            Item::Fn(_) => Kind::Fn,
            Item::Aggregate(_) => Kind::Aggregate,
            Item::Constraint(_) => Kind::Constraint,
            Item::Check(_) => Kind::Check,
            Item::Store(_) => Kind::Store,
            Item::Held(kind, _) => kind,
        }
    }
}

/// The write state an `enable`, `disable` or `deprecate` moves: of a constructor relation, by
/// relation variable and constructor, or of a value-level constructor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StateKey {
    Relation(u64, u64),
    Value(u64),
}

/// An operator with every name it writes resolved to the object it names in the model it
/// applies to. Text it adds stays at its span of the migration's text.
#[derive(Clone, Debug)]
pub(super) enum Op<'o> {
    AddModule(ast::Ident),
    DropModule(ast::Ident),
    /// A declaration of kind `kind` named `name`, at `span`.
    Add {
        kind: Kind,
        name: String,
        span: Span,
    },
    RedefineFn {
        function: u64,
        span: Span,
    },
    Drop(Item),
    Rename {
        obj: Obj,
        to: ast::Ident,
    },
    Stage {
        key: StateKey,
        to: Stage,
        verb: &'static str,
        shown: String,
    },
    /// A field `path` names added to `part`, a type and the constructor whose fields it names,
    /// `None` for the header; with its type and delete action at their spans.
    AddField {
        part: (u64, Option<u64>),
        path: &'o ast::FieldPath,
        ty: Span,
        action: Option<Span>,
        fill: &'o ast::Fill,
    },
    DropField(u64),
    /// Field `field` of `part`, as `path` names it, given the delete action at `action`.
    AlterField {
        field: u64,
        part: (u64, Option<u64>),
        path: &'o ast::FieldPath,
        action: Span,
    },
    AddIndex {
        kind: IndexKind,
        relation: u64,
        fields: &'o [Expr],
        on: &'o ast::RelationRef,
    },
    DropIndex {
        kind: IndexKind,
        relation: u64,
        fields: &'o [Expr],
        on: &'o ast::RelationRef,
    },
    AddDefault {
        relation: u64,
        ctor: Option<&'o ast::Ident>,
        field: &'o ast::Ident,
        value: &'o ast::DefaultValue,
    },
    DropDefault {
        relation: u64,
        ctor: Option<&'o ast::Ident>,
        field: &'o ast::Ident,
    },
    AddComputed {
        relation: u64,
        field: &'o ast::Ident,
        ty: &'o ast::Type,
        expr: &'o Expr,
    },
    RedefineComputed {
        relation: u64,
        field: &'o ast::Ident,
        ty: &'o ast::Type,
        expr: &'o Expr,
    },
    DropComputed {
        relation: u64,
        field: &'o ast::Ident,
    },
    /// A constructor `path` names added to type `ty`.
    AddConstructor {
        ty: u64,
        path: &'o ast::CtorPath,
        record: Option<&'o [ast::FieldType]>,
        place: Option<&'o CtorPlace>,
        joins: &'o [ast::RelationRef],
        arms: &'o [ast::ArmClause],
    },
    RemoveConstructor {
        ty: u64,
        ctor: u64,
    },
    Split {
        ty: u64,
        ctor: u64,
        path: &'o ast::CtorPath,
        into: &'o [ast::CtorDef],
        arms: &'o [ast::Arm],
        clauses: &'o [ast::SplitClause],
    },
    Merge {
        ty: u64,
        sel: &'o CtorSel,
        into: &'o ast::CtorDef,
        arms: Option<&'o [ast::Arm]>,
        clauses: &'o [ast::ArmClause],
    },
    Nest {
        ty: u64,
        sel: &'o CtorSel,
        into: &'o ast::CtorDef,
        clauses: &'o [ast::ArmClause],
    },
    Unnest {
        ty: u64,
        ctor: u64,
        path: &'o ast::CtorPath,
        clauses: &'o [ast::ArmClause],
    },
    /// Field `field` of `part`, as `path` names it, moved to the constructors `sel` selects, or
    /// to the header.
    MoveField {
        field: u64,
        part: (u64, Option<u64>),
        path: &'o ast::FieldPath,
        sel: Option<&'o CtorSel>,
        fill: Option<&'o ast::Fill>,
    },
    Rekey {
        relation: u64,
        key: &'o [ast::Ident],
        carry: &'o [ast::FieldPath],
    },
    Grant(&'o ast::Grant),
    Revoke(&'o ast::Revoke),
    Script(&'o ast::Write),
    Install {
        name: &'o ast::Ident,
        version: &'o str,
        from: &'o str,
    },
    Uninstall(&'o ast::Ident),
    AdoptTzdb {
        release: &'o str,
        from: &'o str,
    },
    Revert {
        migration: &'o str,
        fills: &'o [(ast::FieldPath, ast::Fill)],
    },
}

/// Why an operator does not apply, and the span of the migration's text the refusal is at, when
/// the model names one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Refused {
    why: String,
    at: Option<Span>,
    /// False for a refusal that reports nothing: a name a refused operator poisoned stands
    /// behind every failure.
    reported: bool,
    /// The operator's further failures, each with the span of the migration's text it names,
    /// if it names one.
    more: Vec<(Option<Span>, String)>,
    /// The kind of the diagnostic `why` reports.
    kind: DiagKind,
}

impl Refused {
    pub(super) fn new(why: String) -> Refused {
        Refused {
            why,
            at: None,
            reported: true,
            more: Vec::new(),
            kind: DiagKind::Invalid,
        }
    }

    /// Returns the refusal `why` at `at`.
    pub(super) fn at(at: Span, why: String) -> Refused {
        Refused {
            why,
            at: Some(at),
            reported: true,
            more: Vec::new(),
            kind: DiagKind::Invalid,
        }
    }

    /// Returns a refusal that reports nothing.
    pub(super) fn poisoned() -> Refused {
        Refused {
            why: String::new(),
            at: None,
            reported: false,
            more: Vec::new(),
            kind: DiagKind::Invalid,
        }
    }

    pub(super) fn reported(&self) -> bool {
        self.reported
    }

    pub(super) fn why(&self) -> &str {
        &self.why
    }

    /// Returns the refusal at its own span, else at `operator`, the span of its operator.
    pub(super) fn or_at(self, operator: Span) -> Refused {
        Refused {
            at: Some(self.at.unwrap_or(operator)),
            ..self
        }
    }

    pub(super) fn into_why(self) -> String {
        self.why
    }

    /// Returns the operator's further failures, each with the span it names, if it names one.
    pub(super) fn more(&self) -> &[(Option<Span>, String)] {
        &self.more
    }

    /// Returns the span the refusal names, if it names one.
    pub(super) fn span(&self) -> Option<Span> {
        self.at
    }

    /// Returns the diagnostic of the refusal at `operator` when it names no span.
    pub(super) fn diag(&self, operator: Span) -> Diag {
        Diag {
            span: self.span().unwrap_or(operator),
            message: self.why.clone(),
            kind: self.kind.clone(),
        }
    }
}

/// Why a model gives no result for an operator.
#[derive(Clone, Debug)]
pub(super) enum Fail {
    /// The operator does not apply.
    Refused(Refused),
    /// The model fails to apply the operator, for this reason.
    Broken(String),
}

pub(super) type Res<T> = Result<T, Fail>;

pub(super) fn refuse<T>(why: impl Into<String>) -> Res<T> {
    Err(Fail::Refused(Refused::new(why.into())))
}

/// Refuse the operator with the message and kind of `d`.
pub(super) fn refuse_diag<T>(d: &Diag) -> Res<T> {
    Err(Fail::Refused(Refused {
        kind: d.kind.clone(),
        ..Refused::new(d.message.clone())
    }))
}

/// Refuse the operator for each of `ds` that reports, at its span, reporting nothing when none
/// does.
pub(super) fn refuse_diags<T>(ds: &[Diag]) -> Res<T> {
    let mut reported = ds.iter().filter(|d| d.kind != DiagKind::Poisoned);
    match reported.next() {
        Some(d) => Err(Fail::Refused(Refused {
            more: reported
                .map(|d| (Some(d.span), d.message.clone()))
                .collect(),
            kind: d.kind.clone(),
            ..Refused::at(d.span, d.message.clone())
        })),
        None => Err(Fail::Refused(Refused::poisoned())),
    }
}

/// Refuse the operator for the first of `ds` that reports, `why` writing its message, reporting
/// nothing when none does.
pub(super) fn refuse_first<T>(ds: &[FileDiag], why: impl Fn(&str) -> String) -> Res<T> {
    match ds.iter().find(|d| d.diag.kind != DiagKind::Poisoned) {
        Some(d) => Err(Fail::Refused(Refused {
            kind: d.diag.kind.clone(),
            ..Refused::new(why(&d.diag.message))
        })),
        None => Err(Fail::Refused(Refused::poisoned())),
    }
}

/// Refuse the operator for each of `ds` that reports, reporting nothing when none does.
pub(super) fn refuse_each<T>(ds: &[FileDiag]) -> Res<T> {
    let mut reported = ds.iter().filter(|d| d.diag.kind != DiagKind::Poisoned);
    match reported.next() {
        Some(d) => Err(Fail::Refused(Refused {
            more: reported.map(|d| (None, d.diag.message.clone())).collect(),
            kind: d.diag.kind.clone(),
            ..Refused::new(d.diag.message.clone())
        })),
        None => Err(Fail::Refused(Refused::poisoned())),
    }
}

/// Refuse an operator that checks against the schema the operators before it produce, which
/// fails with `ds`, by the first failure.
pub(super) fn unchecked<T>(ds: &[FileDiag]) -> Res<T> {
    refuse_first(ds, |why| {
        format!(
            "the operator checks against the schema the operators before it produce, which does not check: {why}"
        )
    })
}

/// Refuse the operator for `why`, at `at` of the migration's text.
pub(super) fn refuse_at<T>(at: Span, why: impl Into<String>) -> Res<T> {
    Err(Fail::Refused(Refused::at(at, why.into())))
}

impl Fail {
    /// Returns the reason the failure gives.
    pub(super) fn reason(self) -> String {
        match self {
            Fail::Refused(r) => r.why,
            Fail::Broken(why) => why,
        }
    }
}

pub(super) fn broken<T>(why: impl Into<String>) -> Res<T> {
    Err(Fail::Broken(why.into()))
}

impl Model {
    /// Returns the declaration of kind `kind` named `name` in module `module`; a set's name is
    /// `relation.set`.
    pub(super) fn find(&self, module: &str, kind: Kind, name: &str) -> Option<Item> {
        self.decls
            .iter()
            .filter(|d| d.module == module && Kind::of(d.item) == kind)
            .map(|d| d.item)
            .find(|&item| self.decl_name(item) == name)
    }

    /// Returns the part of type `t` a field path names, `ctor` or the header, as a refusal names
    /// it: `T.C`, or `T` for the header or the constructor of a record.
    pub(super) fn shown_part(&self, t: u64, ctor: Option<u64>) -> String {
        let ty = &self.types[&t].name;
        match ctor.map(|c| &self.ctors[&c].name) {
            Some(c) if c != ty => format!("{ty}.{c}"),
            _ => ty.clone(),
        }
    }

    /// Returns declaration `i` as a refusal names it: its kind and name.
    pub(super) fn shown_decl(&self, i: usize) -> String {
        let item = self.decls[i].item;
        format!("{} `{}`", Kind::of(item).word(), self.decl_name(item))
    }

    /// Returns the name a declaration is found by: a set's as `relation.set`.
    fn decl_name(&self, item: Item) -> String {
        match item {
            Item::Type(t) => self.types[&t].name.clone(),
            Item::Relation(r) => self.relations[&r].name.clone(),
            Item::Set(s) => {
                let set = &self.sets[&s];
                format!("{}.{}", self.relations[&set.relation].name, set.name)
            }
            Item::Fn(f) | Item::Aggregate(f) => self.functions[&f].name.clone(),
            Item::Constraint(c) | Item::Check(c) => self.denials[&c].name.clone(),
            Item::Store(v) => self.store_name(v).to_string(),
            Item::Held(_, k) => self.held[&k].name.clone(),
        }
    }

    pub(super) fn type_named(&self, module: &str, name: &str) -> Option<u64> {
        match self.find(module, Kind::Type, name)? {
            Item::Type(t) => Some(t),
            _ => None,
        }
    }

    pub(super) fn relation_named(&self, module: &str, name: &str) -> Option<u64> {
        match self.find(module, Kind::Relation, name)? {
            Item::Relation(r) => Some(r),
            _ => None,
        }
    }

    /// Returns the type of the rows of relation variable `r`.
    pub(super) fn relation_type(&self, r: u64) -> Option<u64> {
        self.relations[&r]
            .ty
            .objects()
            .find(|o| o.0 == Space::Shared && self.types.contains_key(&o.1))
            .map(|o| o.1)
    }

    pub(super) fn ctor_named(&self, ty: u64, name: &str) -> Option<u64> {
        self.ctors_of(ty)
            .into_iter()
            .find(|c| self.ctors[c].name == name)
    }

    /// Returns the named sets of relation variable `r`.
    pub(super) fn sets_of(&self, r: u64) -> Vec<u64> {
        self.sets
            .iter()
            .filter(|(_, s)| s.relation == r)
            .map(|(&id, _)| id)
            .collect()
    }

    /// Returns the relation variables whose rows are of type `t`.
    pub(super) fn relations_of(&self, t: u64) -> Vec<u64> {
        self.relations
            .keys()
            .copied()
            .filter(|&r| self.relation_type(r) == Some(t))
            .collect()
    }

    /// Returns the fields of `ty` a path names: of the header, or of constructor `ctor`.
    pub(super) fn own_fields(&self, ty: u64, ctor: Option<u64>) -> Vec<u64> {
        match ctor {
            None => self.types[&ty].header.clone().unwrap_or_default(),
            Some(c) => self.ctors[&c].fields.clone().unwrap_or_default(),
        }
    }
}

/// The names of one operator of a migration of `module`, to resolve against `model`.
pub(super) struct Resolving<'a> {
    model: &'a Model,
    module: &'a str,
    /// The migration's text.
    text: &'a str,
}

impl<'a> Resolving<'a> {
    pub(super) fn new(model: &'a Model, module: &'a str) -> Resolving<'a> {
        Resolving {
            model,
            module,
            text: "",
        }
    }

    /// Returns the resolving of the operators of migration text `text`.
    pub(super) fn of(model: &'a Model, module: &'a str, text: &'a str) -> Resolving<'a> {
        Resolving {
            model,
            module,
            text,
        }
    }

    pub(super) fn module_of(&self, q: &QName) -> String {
        q.module
            .as_ref()
            .map_or_else(|| self.module.to_string(), |m| m.text.clone())
    }

    pub(super) fn ty(&self, q: &QName) -> Res<u64> {
        let m = self.module_of(q);
        match self.model.type_named(&m, &q.name.text) {
            Some(t) => Ok(t),
            None => {
                self.unheld(&m, Kind::Type, &q.name.text)?;
                refuse_at(
                    q.span(),
                    format!("module `{m}` declares no type `{}`", q.name.text),
                )
            }
        }
    }

    pub(super) fn relation(&self, q: &QName) -> Res<u64> {
        let m = self.module_of(q);
        match self.model.relation_named(&m, &q.name.text) {
            Some(r) => Ok(r),
            None => {
                self.unheld(&m, Kind::Relation, &q.name.text)?;
                refuse_at(
                    q.span(),
                    format!("module `{m}` declares no relation `{}`", q.name.text),
                )
            }
        }
    }

    /// Refuse by the declarations' first failure when module `m` holds a declaration of kind
    /// `kind` named `name` added while they do not build.
    fn unheld(&self, m: &str, kind: Kind, name: &str) -> Res<()> {
        if !matches!(self.model.find(m, kind, name), Some(Item::Held(..))) {
            return Ok(());
        }
        match self.model.built(&[], &self.model.ahead_inputs()) {
            Err(ds) => unchecked(&ds),
            Ok(_) => broken("a held declaration in declarations that build"),
        }
    }

    fn decl(&self, kind: Kind, q: &QName) -> Res<Item> {
        let m = self.module_of(q);
        match self.model.find(&m, kind, &q.name.text) {
            Some(item) => Ok(item),
            None => refuse_at(
                q.span(),
                format!("module `{m}` declares no {} `{}`", kind.word(), q.name.text),
            ),
        }
    }

    pub(super) fn set(&self, name: &ast::Ident, rel: &QName) -> Res<u64> {
        let m = self.module_of(rel);
        match self
            .model
            .find(&m, Kind::Set, &format!("{}.{}", rel.name.text, name.text))
        {
            Some(Item::Set(s)) => Ok(s),
            _ => refuse_at(
                name.span,
                format!("`{}` declares no named set `{}`", rel.name.text, name.text),
            ),
        }
    }

    fn ctor(&self, path: &ast::CtorPath) -> Res<(u64, u64)> {
        let t = self.ty(&path.ty)?;
        match self.model.ctor_named(t, &path.ctor.text) {
            Some(c) => Ok((t, c)),
            None => refuse_at(
                path.ctor.span,
                format!(
                    "`{}` is not a constructor of `{}`",
                    path.ctor.text, path.ty.name.text
                ),
            ),
        }
    }

    /// Returns the type a field path names and the constructor whose fields it names, `None`
    /// for the header.
    pub(super) fn part(&self, path: &ast::FieldPath) -> Res<(u64, Option<u64>)> {
        let t = self.ty(&path.ty)?;
        let ty = &self.model.types[&t];
        match (&ty.body, &path.ctor) {
            (TypeBody::New { .. }, _) => refuse(format!("newtype `{}` has no fields", ty.name)),
            (TypeBody::Record { ctor }, None) => Ok((t, Some(*ctor))),
            (TypeBody::Record { ctor }, Some(c)) if c.text == ty.name => Ok((t, Some(*ctor))),
            (TypeBody::Ctors(_), None) => Ok((t, None)),
            (TypeBody::Ctors(_), Some(c)) => match self.model.ctor_named(t, &c.text) {
                Some(k) => Ok((t, Some(k))),
                None => refuse_at(
                    c.span,
                    format!("`{}` is not a constructor of `{}`", c.text, ty.name),
                ),
            },
            (TypeBody::Record { .. }, Some(c)) => refuse_at(
                c.span,
                format!("`{}` is not a constructor of `{}`", c.text, ty.name),
            ),
        }
    }

    fn field(&self, path: &ast::FieldPath) -> Res<u64> {
        let (t, c) = self.part(path)?;
        let found = self
            .model
            .own_fields(t, c)
            .into_iter()
            .find(|f| self.model.fields[f].name == path.field.text);
        match found {
            Some(f) => Ok(f),
            None => refuse_at(
                path.field.span,
                format!(
                    "`{}` has no field `{}`",
                    self.model.shown_part(t, c),
                    path.field.text
                ),
            ),
        }
    }

    fn state_key(&self, t: &CtorTarget) -> Res<(StateKey, String)> {
        match t {
            CtorTarget::Relation(rr) => {
                let r = self.relation(&rr.relation)?;
                let c = match &rr.sel {
                    Some(CtorSel::One(c)) => self
                        .model
                        .relation_type(r)
                        .and_then(|t| self.model.ctor_named(t, &c.text))
                        .map(|k| (k, c)),
                    _ => None,
                };
                let Some((k, c)) = c else {
                    return refuse_at(
                        rr.span,
                        "a lifecycle operator names one constructor: `r.C` or `T.C`",
                    );
                };
                Ok((
                    StateKey::Relation(r, k),
                    format!("{}.{}", self.model.relations[&r].name, c.text),
                ))
            }
            CtorTarget::Type(p) => {
                let (t, c) = self.ctor(p)?;
                Ok((
                    StateKey::Value(c),
                    format!("{}.{}", self.model.types[&t].name, p.ctor.text),
                ))
            }
        }
    }

    /// Returns the type and constructor a field path names and the field of them it names.
    fn field_at(&self, path: &ast::FieldPath) -> Res<((u64, Option<u64>), u64)> {
        let (t, c) = self.part(path)?;
        match self.model.field_of(t, c, &path.field.text) {
            Some(f) => Ok(((t, c), f)),
            None => refuse_at(
                path.field.span,
                format!(
                    "`{}` has no field `{}`",
                    self.model.shown_part(t, c),
                    path.field.text
                ),
            ),
        }
    }

    /// Returns `op` with the names it writes resolved, or why it does not apply.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per operator kind; a split by family would need a catch-all arm in each part"
    )]
    pub(super) fn op<'o>(&self, op: &'o OperatorKind) -> Res<Op<'o>> {
        let add = |kind: Kind, name: &str, span: Span| {
            Ok(Op::Add {
                kind,
                name: name.to_string(),
                span,
            })
        };
        let rename = |obj: Obj, to: &ast::Ident| {
            Ok(Op::Rename {
                obj,
                to: to.clone(),
            })
        };
        let stage = |t: &CtorTarget, verb: &'static str, to: Stage| {
            let (key, shown) = self.state_key(t)?;
            Ok(Op::Stage {
                key,
                to,
                verb,
                shown,
            })
        };
        match op {
            OperatorKind::AddModule(m) => Ok(Op::AddModule(m.clone())),
            OperatorKind::DropModule(m) => Ok(Op::DropModule(m.clone())),
            OperatorKind::AddType(t) => add(Kind::Type, &t.name.text, t.span),
            OperatorKind::AddRelation(r) => {
                if r.name.module.is_some() {
                    return refuse_at(
                        r.name.span(),
                        "a relation name is qualified only in the engine's own declarations",
                    );
                }
                add(Kind::Relation, &r.name.name.text, r.span)
            }
            OperatorKind::AddSet(s) => add(
                Kind::Set,
                &format!("{}.{}", s.relation.text, s.name.text),
                s.span,
            ),
            OperatorKind::DefineFn(f) => add(Kind::Fn, &f.name.text, f.span),
            OperatorKind::DefineAggregate(a) => add(Kind::Aggregate, &a.name.text, a.span),
            OperatorKind::AddConstraint(c) => add(Kind::Constraint, &c.name.text, c.span),
            OperatorKind::AddCheck(c) => add(Kind::Check, &c.name.text, c.span),
            OperatorKind::Store(s) => add(Kind::Store, &s.view.name.text, s.span),
            OperatorKind::RedefineFn(f) => {
                match self.model.find(self.module, Kind::Fn, &f.name.text) {
                    Some(Item::Fn(function)) => Ok(Op::RedefineFn {
                        function,
                        span: f.span,
                    }),
                    _ => refuse_at(
                        f.name.span,
                        format!("module `{}` declares no fn `{}`", self.module, f.name.text),
                    ),
                }
            }
            OperatorKind::DropType(q) => Ok(Op::Drop(self.decl(Kind::Type, q)?)),
            OperatorKind::DropRelation(q) => Ok(Op::Drop(self.decl(Kind::Relation, q)?)),
            OperatorKind::DropFn(q) => Ok(Op::Drop(self.decl(Kind::Fn, q)?)),
            OperatorKind::DropAggregate(q) => Ok(Op::Drop(self.decl(Kind::Aggregate, q)?)),
            OperatorKind::DropConstraint(q) => Ok(Op::Drop(self.decl(Kind::Constraint, q)?)),
            OperatorKind::DropCheck(q) => Ok(Op::Drop(self.decl(Kind::Check, q)?)),
            OperatorKind::Unstore(q) => Ok(Op::Drop(self.decl(Kind::Store, q)?)),
            OperatorKind::DropSet(s, r) => Ok(Op::Drop(Item::Set(self.set(s, r)?))),
            OperatorKind::RenameType(q, to) => rename((Space::Shared, self.ty(q)?), to),
            OperatorKind::RenameRelation(q, to) => rename((Space::Shared, self.relation(q)?), to),
            OperatorKind::RenameSet(s, r, to) => rename((Space::Set, self.set(s, r)?), to),
            OperatorKind::RenameConstructor(p, to) => rename((Space::Shared, self.ctor(p)?.1), to),
            OperatorKind::RenameField(p, to) => rename((Space::Field, self.field(p)?), to),
            OperatorKind::Enable(t) => stage(t, "enable", Stage::Enabled),
            OperatorKind::Disable(t) => stage(t, "disable", Stage::Declared),
            OperatorKind::Deprecate(t) => stage(t, "deprecate", Stage::Deprecated),
            OperatorKind::AddField {
                path,
                ty,
                on_delete,
                decl,
                fill,
            } => Ok(Op::AddField {
                part: self.part(path)?,
                path,
                ty: ty.span,
                action: on_delete
                    .as_ref()
                    .map(|_| action_span(self.text, ty.span, *decl)),
                fill,
            }),
            OperatorKind::DropField(path) => Ok(Op::DropField(self.field_at(path)?.1)),
            OperatorKind::AlterField { path, span, .. } => {
                let (part, field) = self.field_at(path)?;
                Ok(Op::AlterField {
                    field,
                    part,
                    path,
                    action: *span,
                })
            }
            OperatorKind::AddUnique(fields, on) => Ok(Op::AddIndex {
                kind: IndexKind::Unique,
                relation: self.relation(&on.relation)?,
                fields,
                on,
            }),
            OperatorKind::AddIndex {
                interval,
                fields,
                on,
            } => Ok(Op::AddIndex {
                kind: index_kind(*interval),
                relation: self.relation(&on.relation)?,
                fields,
                on,
            }),
            OperatorKind::DropUnique(fields, on) => Ok(Op::DropIndex {
                kind: IndexKind::Unique,
                relation: self.relation(&on.relation)?,
                fields,
                on,
            }),
            OperatorKind::DropIndex {
                interval,
                fields,
                on,
            } => Ok(Op::DropIndex {
                kind: index_kind(*interval),
                relation: self.relation(&on.relation)?,
                fields,
                on,
            }),
            OperatorKind::AddDefault {
                relation,
                ctor,
                field,
                value,
            } => Ok(Op::AddDefault {
                relation: self.relation(relation)?,
                ctor: ctor.as_ref(),
                field,
                value,
            }),
            OperatorKind::DropDefault {
                relation,
                ctor,
                field,
            } => Ok(Op::DropDefault {
                relation: self.relation(relation)?,
                ctor: ctor.as_ref(),
                field,
            }),
            OperatorKind::AddComputed {
                relation,
                field,
                ty,
                expr,
            } => Ok(Op::AddComputed {
                relation: self.relation(relation)?,
                field,
                ty,
                expr,
            }),
            OperatorKind::RedefineComputed {
                relation,
                field,
                ty,
                expr,
            } => Ok(Op::RedefineComputed {
                relation: self.relation(relation)?,
                field,
                ty,
                expr,
            }),
            OperatorKind::DropComputed(relation, field) => Ok(Op::DropComputed {
                relation: self.relation(relation)?,
                field,
            }),
            OperatorKind::AddConstructor {
                path,
                record,
                place,
                joins,
                arms,
            } => Ok(Op::AddConstructor {
                ty: self.ty(&path.ty)?,
                path,
                record: record.as_deref(),
                place: place.as_ref(),
                joins,
                arms,
            }),
            OperatorKind::RemoveConstructor(path) => {
                let t = self.ty(&path.ty)?;
                match self.model.ctor_named(t, &path.ctor.text) {
                    Some(ctor) => Ok(Op::RemoveConstructor { ty: t, ctor }),
                    None => refuse(format!(
                        "`{}` is not a constructor of `{}`",
                        path.ctor.text, self.model.types[&t].name
                    )),
                }
            }
            OperatorKind::SplitConstructor {
                path,
                into,
                arms,
                clauses,
            } => {
                let (ty, ctor) = self.ctor(path)?;
                Ok(Op::Split {
                    ty,
                    ctor,
                    path,
                    into,
                    arms,
                    clauses,
                })
            }
            OperatorKind::MergeConstructor {
                ty,
                sel,
                into,
                arms,
                clauses,
            } => Ok(Op::Merge {
                ty: self.ty(ty)?,
                sel,
                into,
                arms: arms.as_deref(),
                clauses,
            }),
            OperatorKind::NestConstructor {
                ty,
                sel,
                into,
                clauses,
            } => Ok(Op::Nest {
                ty: self.ty(ty)?,
                sel,
                into,
                clauses,
            }),
            OperatorKind::UnnestConstructor(path, clauses) => {
                let t = self.ty(&path.ty)?;
                match self.model.ctor_named(t, &path.ctor.text) {
                    Some(ctor) => Ok(Op::Unnest {
                        ty: t,
                        ctor,
                        path,
                        clauses,
                    }),
                    None => refuse(format!(
                        "`{}` is not a constructor of `{}`",
                        path.ctor.text, self.model.types[&t].name
                    )),
                }
            }
            OperatorKind::MoveField {
                path,
                to,
                sel,
                fill,
            } => {
                self.ty(&path.ty)?;
                if to.name.text != path.ty.name.text
                    || self.module_of(to) != self.module_of(&path.ty)
                {
                    return refuse_at(to.span(), "a field moves within its own type");
                }
                let (part, field) = self.field_at(path)?;
                Ok(Op::MoveField {
                    field,
                    part,
                    path,
                    sel: sel.as_ref(),
                    fill: fill.as_ref(),
                })
            }
            OperatorKind::Rekey {
                relation,
                key,
                carry,
            } => Ok(Op::Rekey {
                relation: self.relation(relation)?,
                key,
                carry,
            }),
            OperatorKind::Grant(g) => Ok(Op::Grant(g)),
            OperatorKind::Revoke(r) => Ok(Op::Revoke(r)),
            OperatorKind::Script(w) => Ok(Op::Script(w)),
            OperatorKind::Install {
                name,
                version,
                from,
            } => Ok(Op::Install {
                name,
                version,
                from,
            }),
            OperatorKind::Uninstall(m) => Ok(Op::Uninstall(m)),
            OperatorKind::AdoptTzdb { release, from } => Ok(Op::AdoptTzdb { release, from }),
            OperatorKind::Revert { migration, fills } => Ok(Op::Revert { migration, fills }),
        }
    }
}

fn index_kind(interval: bool) -> IndexKind {
    if interval {
        IndexKind::Interval
    } else {
        IndexKind::Plain
    }
}

/// Returns the span of a field's delete action, after its type at `ty` within its declaration
/// `decl` of `text`.
fn action_span(text: &str, ty: Span, decl: Span) -> Span {
    let from = ty.end as usize;
    let toks: Vec<_> = lex(&text[from..decl.end as usize])
        .0
        .into_iter()
        .filter(|t| t.tok != Tok::Eof)
        .collect();
    let start = toks
        .get(2)
        .map_or(decl.end as usize, |t| from + t.span.start as usize);
    Span::new(start, decl.end as usize)
}
