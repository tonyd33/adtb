use super::*;
use crate::ast::{self, CtorSel, CtorTarget, OperatorKind, QName};
use crate::schema::Stage;
use crate::span::{DiagKind, FileDiag, Span};

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
pub(super) enum Op {
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
}

impl Refused {
    pub(super) fn new(why: String) -> Refused {
        Refused {
            why,
            at: None,
            reported: true,
            more: Vec::new(),
        }
    }

    /// Returns the refusal `why` at `at`.
    pub(super) fn at(at: Span, why: String) -> Refused {
        Refused {
            why,
            at: Some(at),
            reported: true,
            more: Vec::new(),
        }
    }

    /// Returns a refusal that reports nothing.
    pub(super) fn poisoned() -> Refused {
        Refused {
            why: String::new(),
            at: None,
            reported: false,
            more: Vec::new(),
        }
    }

    pub(super) fn reported(&self) -> bool {
        self.reported
    }

    pub(super) fn why(&self) -> &str {
        &self.why
    }

    /// Returns the span the refusal names, if it names one.
    pub(super) fn span(&self) -> Option<Span> {
        self.at
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

/// Refuse the operator for each of `ds` that reports, at its span, reporting nothing when none
/// does.
pub(super) fn refuse_diags<T>(ds: &[crate::span::Diag]) -> Res<T> {
    let mut reported = ds.iter().filter(|d| d.kind != DiagKind::Poisoned);
    match reported.next() {
        Some(d) => Err(Fail::Refused(Refused {
            more: reported
                .map(|d| (Some(d.span), d.message.clone()))
                .collect(),
            ..Refused::at(d.span, d.message.clone())
        })),
        None => Err(Fail::Refused(Refused::poisoned())),
    }
}

/// Refuse the operator for the first of `ds` that reports, `why` writing its message, reporting
/// nothing when none does.
pub(super) fn refuse_first<T>(ds: &[FileDiag], why: impl Fn(&str) -> String) -> Res<T> {
    match ds.iter().find(|d| d.diag.kind != DiagKind::Poisoned) {
        Some(d) => refuse(why(&d.diag.message)),
        None => Err(Fail::Refused(Refused::poisoned())),
    }
}

/// Refuse the operator for each of `ds` that reports, reporting nothing when none does.
pub(super) fn refuse_each<T>(ds: &[FileDiag]) -> Res<T> {
    let mut reported = ds.iter().filter(|d| d.diag.kind != DiagKind::Poisoned);
    match reported.next() {
        Some(d) => Err(Fail::Refused(Refused {
            more: reported.map(|d| (None, d.diag.message.clone())).collect(),
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
}

impl<'a> Resolving<'a> {
    pub(super) fn new(model: &'a Model, module: &'a str) -> Resolving<'a> {
        Resolving { model, module }
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

    /// Returns `op` with the names it writes resolved, or why it does not apply.
    pub(super) fn op(&self, op: &OperatorKind) -> Res<Op> {
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
            other => broken(format!(
                "an operator applied before its names resolve: {}",
                operator_name(other)
            )),
        }
    }
}

/// Returns the name of an operator's kind, as a skip is counted under.
pub(super) fn operator_name(op: &OperatorKind) -> &'static str {
    match op {
        OperatorKind::AddModule(_) => "add module",
        OperatorKind::DropModule(_) => "drop module",
        OperatorKind::AddType(_) => "add type",
        OperatorKind::DropType(_) => "drop type",
        OperatorKind::RenameType(..) => "rename type",
        OperatorKind::AddRelation(_) => "add relation",
        OperatorKind::DropRelation(_) => "drop relation",
        OperatorKind::RenameRelation(..) => "rename relation",
        OperatorKind::AddSet(_) => "add set",
        OperatorKind::DropSet(..) => "drop set",
        OperatorKind::RenameSet(..) => "rename set",
        OperatorKind::AddConstructor { .. } => "add constructor",
        OperatorKind::RemoveConstructor(_) => "remove constructor",
        OperatorKind::RenameConstructor(..) => "rename constructor",
        OperatorKind::SplitConstructor { .. } => "split",
        OperatorKind::MergeConstructor { .. } => "merge",
        OperatorKind::NestConstructor { .. } => "nest",
        OperatorKind::UnnestConstructor(..) => "unnest",
        OperatorKind::AddField { .. } => "add field",
        OperatorKind::AlterField { .. } => "alter field",
        OperatorKind::DropField(_) => "drop field",
        OperatorKind::RenameField(..) => "rename field",
        OperatorKind::MoveField { .. } => "move field",
        OperatorKind::AddUnique(..) => "add unique",
        OperatorKind::DropUnique(..) => "drop unique",
        OperatorKind::AddIndex { .. } => "add index",
        OperatorKind::DropIndex { .. } => "drop index",
        OperatorKind::Rekey { .. } => "rekey",
        OperatorKind::AddDefault { .. } => "add default",
        OperatorKind::DropDefault { .. } => "drop default",
        OperatorKind::DefineFn(_) => "define fn",
        OperatorKind::RedefineFn(_) => "redefine fn",
        OperatorKind::DropFn(_) => "drop fn",
        OperatorKind::AddConstraint(_) => "add constraint",
        OperatorKind::DropConstraint(_) => "drop constraint",
        OperatorKind::AddCheck(_) => "add check",
        OperatorKind::DropCheck(_) => "drop check",
        OperatorKind::Store(_) => "store",
        OperatorKind::Unstore(_) => "unstore",
        OperatorKind::AddComputed { .. } => "add computed",
        OperatorKind::RedefineComputed { .. } => "redefine computed",
        OperatorKind::DropComputed(..) => "drop computed",
        OperatorKind::DefineAggregate(_) => "define aggregate",
        OperatorKind::DropAggregate(_) => "drop aggregate",
        OperatorKind::Enable(_) => "enable",
        OperatorKind::Disable(_) => "disable",
        OperatorKind::Deprecate(_) => "deprecate",
        OperatorKind::Revert { .. } => "revert",
        OperatorKind::Script(_) => "script",
        OperatorKind::Install { .. } => "install",
        OperatorKind::Uninstall(_) => "uninstall",
        OperatorKind::AdoptTzdb { .. } => "adopt tzdb",
        OperatorKind::Grant(_) => "grant",
        OperatorKind::Revoke(_) => "revoke",
    }
}
