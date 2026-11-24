use std::ops::Range;

use super::run::{Built, Decls, Late, Run};
use super::*;
use crate::ast::{self, Operator, OperatorKind, RelationClause, SplitClause};
use crate::canon::Names;
use crate::evolve::text::ctor_extent;
use crate::evolve::{Clause, IndexKind, Resolved, ResolvedOp};
use crate::lex::{Tok, lex};
use crate::span::{Diag, FileId, Span};

/// The names of the migration's text that the operators applied so far copy into the
/// declarations.
#[derive(Clone, Debug, Default)]
pub(super) struct Naming {
    names: Names,
    /// The copies not named yet: each range of the migration's text, where it was copied, and
    /// the names the last build that held it resolved in it, `None` when no build did.
    pending: Vec<(Range<usize>, At, Option<Names>)>,
    /// The range of the migration's text each arm an operator wrote copies, by the arm's key.
    arms: BTreeMap<u64, Range<usize>>,
    /// The copies a later operator rewrote before they were named.
    missing: Vec<Diag>,
    /// The migration's text.
    text: std::sync::Arc<str>,
    /// Each constructor a split of the migration replaced, by name, with the names of its
    /// products.
    split: Vec<(String, Vec<String>)>,
}

/// Where a copy of the migration's text is in the declarations.
#[derive(Clone, Copy, Debug)]
pub(super) enum At {
    /// The whole declaration.
    Decl(Item),
    /// The declaration added as [`Item::Held`] with this key, until it lifts.
    Held(u64),
    /// A field from its name through its delete action, or its delete action alone.
    Field { field: u64, action_only: bool },
    /// The fields of a unique or index.
    Index { relation: u64, index: u64 },
    /// The value of a default.
    Default { relation: u64, default: u64 },
    /// The type, or the expression, of a computed field.
    Computed {
        relation: u64,
        computed: u64,
        expr: bool,
    },
    /// Constructors `first` through `last` of a type, from the name of `first` through the record
    /// of `last` when `record`.
    Ctors { first: u64, last: u64, record: bool },
}

impl Naming {
    /// Returns the naming of the copies of migration text `text`, none named yet.
    pub(super) fn of(text: &str) -> Naming {
        Naming {
            text: text.into(),
            ..Naming::default()
        }
    }

    /// Returns the names of the copies, failing with each a later operator rewrote before it was
    /// named.
    pub(super) fn into_names(self) -> Result<Names, Vec<Diag>> {
        if self.missing.is_empty() {
            Ok(self.names)
        } else {
            Err(self.missing)
        }
    }

    /// Note that the arm of key `key` copies `from` of the migration's text.
    pub(super) fn arm(&mut self, key: u64, from: Range<usize>) {
        self.arms.insert(key, from);
    }
}

/// Copy into `out` the names `names` holds at each token of `range` of `text`, onto the token of
/// `range` of `migration` at the same place; only at places `out` does not name yet when
/// `unnamed`. Push onto `missing` a refusal at `from` when the ranges do not hold the same tokens.
///
/// A name of `migration` of a constructor `split` replaced may stand where `text` holds its
/// products, one or several between `|`, parenthesized or not; it names them all.
fn copy_tokens(
    (out, missing): (&mut Names, &mut Vec<Diag>),
    names: &Names,
    (text, range): (&str, Range<usize>),
    (migration, from): (&str, Range<usize>),
    unnamed: bool,
    split: &[(String, Vec<String>)],
) {
    let toks = |s: &str, r: &Range<usize>| -> Vec<(String, Range<usize>)> {
        lex(&s[r.clone()])
            .0
            .into_iter()
            .filter(|t| t.tok != Tok::Eof)
            .map(|t| {
                let x = t.span.range();
                (
                    s[r.start + x.start..r.start + x.end].to_string(),
                    r.start + x.start..r.start + x.end,
                )
            })
            .collect()
    };
    let (theirs, ours) = (toks(text, &range), toks(migration, &from));
    let mut pairs = Vec::new();
    let mut retyped = Vec::new();
    let mut i = 0;
    for (word, a) in &ours {
        if theirs.get(i).is_some_and(|(t, _)| t == word) {
            pairs.push((theirs[i].1.clone(), a.clone()));
            i += 1;
            continue;
        }
        let Some((_, products)) = split.iter().find(|(c, _)| c == word) else {
            i = usize::MAX;
            break;
        };
        match products_at(&theirs[i.min(theirs.len())..], products) {
            Some((spans, took)) => {
                retyped.push((spans, a.clone()));
                i += took;
            }
            None => {
                i = usize::MAX;
                break;
            }
        }
    }
    if i != theirs.len() {
        missing.push(Diag::new(
            Span::new(from.start, from.end),
            "the declarations do not hold this operand as written, so its names do not resolve",
        ));
        return;
    }
    for (b, a) in pairs {
        if unnamed {
            out.copy_unnamed(names, b, a.start);
        } else {
            out.copy(names, b, a.start);
        }
    }
    for (spans, a) in retyped {
        out.gather(names, &spans, a);
    }
}

/// Returns the range of each product name `toks` starts with, as `P`, `P | Q` or `(P | Q)` with
/// every name one of `products`, and the number of tokens they take.
fn products_at(
    toks: &[(String, Range<usize>)],
    products: &[String],
) -> Option<(Vec<Range<usize>>, usize)> {
    let open = toks.first().is_some_and(|(t, _)| t == "(");
    let mut at = usize::from(open);
    let mut spans = Vec::new();
    loop {
        let (name, span) = toks.get(at)?;
        if !products.contains(name) {
            return None;
        }
        spans.push(span.clone());
        at += 1;
        if toks.get(at).is_none_or(|(t, _)| t != "|") {
            break;
        }
        at += 1;
    }
    if open {
        if toks.get(at)?.0 != ")" {
            return None;
        }
        at += 1;
    }
    Some((spans, at))
}

/// Returns the declaration operator `op` added or redefined.
fn added_item(op: &Resolved) -> Option<Item> {
    Some(match op {
        Resolved::AddType { sum, .. } => Item::Type(*sum),
        Resolved::AddRelation { relation, .. } => Item::Relation(*relation),
        Resolved::AddSet { set, .. } => Item::Set(*set),
        Resolved::DefineFn { function, .. } | Resolved::RedefineFn { function, .. } => {
            Item::Fn(*function)
        }
        Resolved::DefineAggregate { function, .. } => Item::Aggregate(*function),
        Resolved::AddConstraint { constraint, .. } => Item::Constraint(*constraint),
        Resolved::AddCheck { constraint, .. } => Item::Check(*constraint),
        Resolved::Store { view, .. } => Item::Store(*view),
        _ => return None,
    })
}

/// Returns whether file `file` of `built`, whose names are `names`, writes a name `built` does not
/// resolve to an object: one it does not declare, or a relation variable a later operator adds.
fn waits(built: &Built, file: FileId, names: &Names) -> bool {
    !names.all_resolve() || !built.ahead_ranges(file).is_empty()
}

impl Model {
    /// Name the copies of the migration's text operator `op` made, resolved as `resolved`, in the
    /// declarations it leaves, and those of earlier operators still unnamed once those
    /// declarations resolve every name the copy's declaration writes.
    pub(super) fn name_copies(&mut self, run: &mut Run, op: &Operator, resolved: &ResolvedOp) {
        if let OperatorKind::SplitConstructor { path, into, .. } = &op.kind {
            let products = into.iter().map(|c| c.name.text.clone()).collect();
            self.naming.split.push((path.ctor.text.clone(), products));
        }
        let mut unpaired = Vec::new();
        let made = self.copies_of(run, op, resolved, &mut unpaired);
        self.naming.missing.extend(unpaired);
        let Ok((built, _)) = self.built(&[], &run.ahead(self)) else {
            self.naming
                .pending
                .extend(made.into_iter().map(|(from, at)| (from, at, None)));
            return;
        };
        let mut naming = std::mem::take(&mut self.naming);
        for (from, at) in made {
            match self.span_of(&built, at) {
                Some((file, span)) => {
                    let names = built.names(file);
                    copy_tokens(
                        (&mut naming.names, &mut naming.missing),
                        &names,
                        (built.text(file), span.range()),
                        (run.text(), from.clone()),
                        false,
                        &naming.split,
                    );
                    if waits(&built, file, &names) {
                        naming.pending.push((from, at, Some(Names::default())));
                    }
                }
                None => naming.pending.push((from, at, None)),
            }
        }
        self.name_pending(&mut naming, &built, run.text(), false);
        self.naming = naming;
    }

    /// Name the copies still unnamed in the declarations the migration leaves.
    pub(super) fn name_late(&mut self, run: &mut Run) {
        if self.naming.pending.is_empty() {
            return;
        }
        let mut naming = std::mem::take(&mut self.naming);
        match self.built(&[], &[]) {
            Ok((built, _)) => self.name_pending(&mut naming, &built, run.text(), true),
            Err(_) => naming
                .missing
                .extend(naming.pending.drain(..).map(|(from, ..)| {
                Diag::new(
                    Span::new(from.start, from.end),
                    "the migration's result does not build, so this operand's names do not resolve",
                )
            })),
        }
        self.naming = naming;
    }

    /// Name each pending copy of `naming` whose declaration `built` resolves every name of, or
    /// every one when `last`, and hold for the others the names `built` resolves in them. Name
    /// each whose place is gone by the names held for it, noting it as missing when none are.
    fn name_pending(&self, naming: &mut Naming, built: &Built, migration: &str, last: bool) {
        let mut kept = Vec::new();
        for (from, at, named) in std::mem::take(&mut naming.pending) {
            let Some((file, span)) = self.span_of(built, at) else {
                match named {
                    _ if !self.gone(at) => kept.push((from, at, named)),
                    Some(names) => naming.names.extend(names),
                    None => naming.missing.push(Diag::new(
                        Span::new(from.start, from.end),
                        "a later operator of the migration rewrites the declaration this operand was written into before any build named it, so its names do not resolve",
                    )),
                }
                continue;
            };
            let names = built.names(file);
            if !last && waits(built, file, &names) {
                let (mut held, mut unpaired) = (Names::default(), Vec::new());
                copy_tokens(
                    (&mut held, &mut unpaired),
                    &names,
                    (built.text(file), span.range()),
                    (migration, from.clone()),
                    false,
                    &naming.split,
                );
                kept.push((from, at, unpaired.is_empty().then_some(held).or(named)));
                continue;
            }
            copy_tokens(
                (&mut naming.names, &mut naming.missing),
                &names,
                (built.text(file), span.range()),
                (migration, from),
                false,
                &naming.split,
            );
        }
        naming.pending = kept;
    }

    /// Name arm `key`, which `built` holds at `range` of file `file`, when an operator of the
    /// migration wrote it.
    pub(super) fn name_arm(&mut self, built: &Built, key: u64, file: FileId, range: Range<usize>) {
        let Some(from) = self.naming.arms.remove(&key) else {
            return;
        };
        let names = built.names(file);
        let text = self.naming.text.clone();
        let naming = &mut self.naming;
        copy_tokens(
            (&mut naming.names, &mut naming.missing),
            &names,
            (built.text(file), range),
            (&text, from),
            false,
            &naming.split,
        );
    }

    /// Returns whether the place `at` names is gone from the model.
    fn gone(&self, at: At) -> bool {
        match at {
            At::Decl(item) => self.decl_at(item).is_none(),
            At::Held(key) => {
                !self.held.contains_key(&key)
                    && self
                        .lifted
                        .get(&key)
                        .is_none_or(|&i| self.decl_at(i).is_none())
            }
            At::Field { field, .. } => !self.fields.contains_key(&field),
            At::Index { relation, .. }
            | At::Default { relation, .. }
            | At::Computed { relation, .. } => !self.relations.contains_key(&relation),
            At::Ctors { first, last, .. } => {
                !self.ctors.contains_key(&first) || !self.ctors.contains_key(&last)
            }
        }
    }

    /// Returns the file and span the copy at `at` takes in `built`, which the model's
    /// declarations built.
    pub(super) fn span_of(&self, built: &impl Decls, at: At) -> Option<(FileId, Span)> {
        match at {
            At::Decl(item) => {
                let file = FileId(self.decl_at(item)? as u32);
                Some((file, built.decl(file)?.span()))
            }
            At::Held(key) => {
                let item = match self.lifted.get(&key) {
                    Some(&item) => item,
                    None => {
                        self.decls
                            .iter()
                            .find(|d| matches!(d.item, Item::Held(_, k) if k == key))?
                            .item
                    }
                };
                self.span_of(built, At::Decl(item))
            }
            At::Field { field, action_only } => {
                let t = self.field_type(field)?;
                let file = FileId(self.decl_at(Item::Type(t))? as u32);
                let owner = self.fields.get(&field)?.owner;
                let ctor = self.ctors.get(&owner).map(|c| c.name.clone());
                let f =
                    Self::field_written(built, file, ctor.as_deref(), &self.fields[&field].name)
                        .ok()?;
                let span = match (&f.on_delete, action_only) {
                    (Some(a), true) => a.span,
                    (None, true) => return None,
                    (a, false) => Span {
                        start: f.name.span.start,
                        end: a.as_ref().map_or(f.ty.span.end, |a| a.span.end),
                    },
                };
                Some((file, span))
            }
            At::Index { relation, index } => {
                let (file, k) = self.clause_place(
                    relation,
                    |c| matches!(c, ClauseM::Index { id: Some(x), .. } if *x == index),
                )?;
                let (RelationClause::Unique(fs, ..) | RelationClause::Index { fields: fs, .. }) =
                    Self::clause_written(built, file, k).ok()?
                else {
                    return None;
                };
                Some((file, fs.first()?.span.to(fs.last()?.span)))
            }
            At::Default { relation, default } => {
                let (file, k) = self.clause_place(relation, |c| {
                    matches!(c, ClauseM::Default { ids, .. } if ids.iter().any(|&(_, d)| d == default))
                })?;
                match Self::clause_written(built, file, k).ok()? {
                    RelationClause::Default {
                        value: ast::DefaultValue::Expr(e),
                        ..
                    } => Some((file, e.span)),
                    _ => None,
                }
            }
            At::Computed {
                relation,
                computed,
                expr,
            } => {
                let (file, k) = self.clause_place(
                    relation,
                    |c| matches!(c, ClauseM::Computed { id: Some(x), .. } if *x == computed),
                )?;
                match Self::clause_written(built, file, k).ok()? {
                    RelationClause::Computed(_, t, e) => {
                        Some((file, if expr { e.span } else { t.span }))
                    }
                    _ => None,
                }
            }
            At::Ctors {
                first,
                last,
                record,
            } => {
                let t = self.ctors.get(&first)?.ty;
                let file = FileId(self.decl_at(Item::Type(t))? as u32);
                let Some(ast::Decl::Type(td)) = built.decl(file) else {
                    return None;
                };
                let ast::TypeBody::Ctors(cs) = &td.body else {
                    return None;
                };
                let find = |c: u64| cs.iter().find(|x| x.name.text == self.ctors[&c].name);
                let (a, z) = (find(first)?, find(self.ctors.get(&last).map(|_| last)?)?);
                let end = ctor_extent(built.text(file), &z.name, record).end;
                Some((file, Span::new(a.name.span.start as usize, end)))
            }
        }
    }

    /// Returns the file of relation variable `r`'s declaration and the position of its first
    /// clause `f` holds.
    fn clause_place(&self, r: u64, f: impl Fn(&ClauseM) -> bool) -> Option<(FileId, usize)> {
        let file = FileId(self.decl_at(Item::Relation(r))? as u32);
        let k = self.relations.get(&r)?.clauses.iter().position(f)?;
        Some((file, k))
    }

    /// Returns each copy of the migration's text operator `op`, resolved as `resolved`, made: the
    /// range it copies and where it is. Push onto `missing` a refusal of each copy whose place is
    /// unknown.
    pub(super) fn copies_of(
        &self,
        run: &mut Run,
        op: &Operator,
        resolved: &ResolvedOp,
        missing: &mut Vec<Diag>,
    ) -> Vec<(Range<usize>, At)> {
        let text = run.text();
        let held = run.held();
        let range = |s: Span| s.range();
        let decl = |span: Span| -> Vec<(Range<usize>, At)> {
            let at = match held {
                Some(key) => At::Held(key),
                None => match added_item(&resolved.op) {
                    Some(item) => At::Decl(item),
                    None => return Vec::new(),
                },
            };
            vec![(range(span), at)]
        };
        match (&op.kind, &resolved.op) {
            (OperatorKind::AddType(t), _) => decl(t.span),
            (OperatorKind::AddRelation(r), _) => decl(r.span),
            (OperatorKind::AddSet(s), _) => decl(s.span),
            (OperatorKind::DefineFn(f) | OperatorKind::RedefineFn(f), _) => decl(f.span),
            (OperatorKind::DefineAggregate(a), _) => decl(a.span),
            (OperatorKind::AddConstraint(c), _) => decl(c.span),
            (OperatorKind::AddCheck(c), _) => decl(c.span),
            (OperatorKind::Store(s), _) => decl(s.span),
            (OperatorKind::AddField { decl, .. }, Resolved::AddField { field, .. }) => vec![(
                range(*decl),
                At::Field {
                    field: *field,
                    action_only: false,
                },
            )],
            (OperatorKind::AlterField { span, .. }, Resolved::AlterField { field, .. }) => vec![(
                range(*span),
                At::Field {
                    field: *field,
                    action_only: true,
                },
            )],
            (
                OperatorKind::AddUnique(fields, _) | OperatorKind::AddIndex { fields, .. },
                Resolved::AddUnique {
                    index, relation, ..
                }
                | Resolved::AddIndex {
                    index, relation, ..
                },
            ) => match (fields.first(), fields.last()) {
                (Some(a), Some(z)) => vec![(
                    a.span.start as usize..z.span.end as usize,
                    At::Index {
                        relation: *relation,
                        index: *index,
                    },
                )],
                _ => Vec::new(),
            },
            (
                OperatorKind::AddDefault {
                    value: ast::DefaultValue::Expr(e),
                    ..
                },
                Resolved::AddDefault {
                    default, relation, ..
                },
            ) => vec![(
                range(e.span),
                At::Default {
                    relation: *relation,
                    default: *default,
                },
            )],
            (
                OperatorKind::AddComputed {
                    relation: _,
                    ty,
                    expr,
                    ..
                }
                | OperatorKind::RedefineComputed { ty, expr, .. },
                Resolved::AddComputed { computed, .. }
                | Resolved::RedefineComputed { computed, .. },
            ) => {
                let Some(relation) = self.computed_relation(*computed) else {
                    return Vec::new();
                };
                [(ty.span, false), (expr.span, true)]
                    .into_iter()
                    .map(|(s, expr)| {
                        (
                            range(s),
                            At::Computed {
                                relation,
                                computed: *computed,
                                expr,
                            },
                        )
                    })
                    .collect()
            }
            (
                OperatorKind::AddConstructor { path, record, .. },
                Resolved::AddConstructor { ctor, .. },
            ) => vec![(
                ctor_extent(text, &path.ctor, record.is_some()),
                At::Ctors {
                    first: *ctor,
                    last: *ctor,
                    record: record.is_some(),
                },
            )],
            (
                OperatorKind::SplitConstructor { .. }
                | OperatorKind::MergeConstructor { .. }
                | OperatorKind::NestConstructor { .. },
                _,
            ) => self.made_copies(run, op, resolved, missing),
            _ => Vec::new(),
        }
    }

    /// Returns each copy of the migration's text split, merge or nest operator `op`, resolved as
    /// `resolved`, made: the constructors it makes and the fields of a split's replacement
    /// clauses. Push onto `missing` a refusal at `op` when its clauses do not match those it
    /// resolved to one for one.
    fn made_copies(
        &self,
        run: &mut Run,
        op: &Operator,
        resolved: &ResolvedOp,
        missing: &mut Vec<Diag>,
    ) -> Vec<(Range<usize>, At)> {
        let ctor = |t: Option<u64>, d: &ast::CtorDef| -> Option<(Range<usize>, At)> {
            let c = self
                .ctors_of(t?)
                .into_iter()
                .find(|c| self.ctors[c].name == d.name.text)?;
            let record = d.record.is_some();
            Some((
                ctor_extent(run.text(), &d.name, record),
                At::Ctors {
                    first: c,
                    last: c,
                    record,
                },
            ))
        };
        let type_of = |q: &ast::QName| {
            let module = q.module.as_ref().map_or(run.module(), |m| m.text.as_str());
            self.type_named(module, &q.name.text)
        };
        match (&op.kind, &resolved.op) {
            (
                OperatorKind::SplitConstructor {
                    path,
                    into,
                    clauses,
                    ..
                },
                Resolved::Split { .. },
            ) => {
                let t = type_of(&path.ty);
                let mut out: Vec<_> = into.iter().filter_map(|d| ctor(t, d)).collect();
                let made: Vec<&Clause> = resolved
                    .clauses
                    .iter()
                    .filter(|c| matches!(c, Clause::Arm { .. } | Clause::Replacement { .. }))
                    .collect();
                let paired = made.len() == clauses.len()
                    && clauses.iter().zip(&made).all(|(w, c)| match (w, c) {
                        (SplitClause::Arm(_), Clause::Arm { .. }) => true,
                        (SplitClause::Unique(..), Clause::Replacement { kind, .. }) => {
                            *kind == IndexKind::Unique
                        }
                        (SplitClause::Index { interval, .. }, Clause::Replacement { kind, .. }) => {
                            *kind
                                == if *interval {
                                    IndexKind::Interval
                                } else {
                                    IndexKind::Plain
                                }
                        }
                        _ => false,
                    });
                if !paired {
                    missing.push(Diag::new(
                        op.span,
                        "the split's clauses do not resolve one for one, so their names do not resolve",
                    ));
                    return out;
                }
                for (w, c) in clauses.iter().zip(made) {
                    let (
                        SplitClause::Unique(fs, _) | SplitClause::Index { fields: fs, .. },
                        Clause::Replacement { index, .. },
                    ) = (w, c)
                    else {
                        continue;
                    };
                    let (Some(a), Some(z), Some(relation)) =
                        (fs.first(), fs.last(), self.index_relation(*index))
                    else {
                        continue;
                    };
                    out.push((
                        a.span.start as usize..z.span.end as usize,
                        At::Index {
                            relation,
                            index: *index,
                        },
                    ));
                }
                out
            }
            (
                OperatorKind::MergeConstructor { ty, into, .. }
                | OperatorKind::NestConstructor { ty, into, .. },
                Resolved::Merge { .. } | Resolved::Nest { .. },
            ) => ctor(type_of(ty), into).into_iter().collect(),
            _ => Vec::new(),
        }
    }

    /// Returns the relation variable holding computed field `computed`.
    fn computed_relation(&self, computed: u64) -> Option<u64> {
        self.relations.iter().find_map(|(&r, rel)| {
            rel.clauses
                .iter()
                .any(|c| matches!(c, ClauseM::Computed { id: Some(x), .. } if *x == computed))
                .then_some(r)
        })
    }

    /// Returns the relation variable holding unique or index `index`.
    fn index_relation(&self, index: u64) -> Option<u64> {
        self.relations.iter().find_map(|(&r, rel)| {
            rel.clauses
                .iter()
                .any(|c| matches!(c, ClauseM::Index { id: Some(x), .. } if *x == index))
                .then_some(r)
        })
    }
}

impl Run<'_> {
    /// Record the names `built` gives each of `held`, expressions of file `file`, onto the
    /// expression of `written`, of the migration's text, at the same place.
    pub(super) fn read(
        &mut self,
        built: &Built,
        file: FileId,
        held: &[ast::Expr],
        written: &[ast::Expr],
    ) {
        let names = built.names(file);
        let (mut out, mut missing) = (Names::default(), Vec::new());
        for (h, w) in held.iter().zip(written) {
            copy_tokens(
                (&mut out, &mut missing),
                &names,
                (built.text(file), h.span.range()),
                (self.text(), w.span.range()),
                false,
                &[],
            );
        }
        self.note_names(out);
        self.miss(missing);
    }

    /// Returns the key of the declaration the operator being applied held, when it held one.
    pub(super) fn held(&self) -> Option<u64> {
        self.lates().iter().find_map(|&l| match l {
            Late::Held(key) => Some(key),
            _ => None,
        })
    }
}
