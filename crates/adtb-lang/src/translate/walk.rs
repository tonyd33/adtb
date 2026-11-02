use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use crate::evolve::Resolved;
use crate::kernel::{Agg, Conflict, Kind, Prim, Terms, Tm, Var};
use crate::schema::DefaultValue;
use crate::types::*;

use super::{Given, Map, Pinned, R, carries, forms, refused};

/// Where a term is the value a write stores: the relation written, at the later version, and
/// the row it replaces, when it replaces one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Site {
    rel: RelId,
    old: Option<Var>,
}

/// A field a write's value lacks: field `name` of type `ty` of constructor `k` of `def`, at the
/// later version.
pub(super) struct Lacked<'s> {
    def: TypeId,
    k: u32,
    name: &'s str,
    ty: &'s Ty,
}

/// One pass over an arena of terms at one version, building terms at the later version.
pub(super) struct Walk<'p, 'a> {
    p: &'p Pinned<'a>,
    map: &'p Map<'a>,
    src: &'p Terms,
    vars: HashMap<Var, Var>,
    memo: HashMap<(Tm, Option<Site>), Tm>,
    /// The rows a fill reads, each a constructor value under construction.
    rows: HashMap<Var, Rc<Given>>,
    /// The `let`-bound variables an insert stores as a row, with the relation of the insert the
    /// `let` builds its value for.
    stored: HashMap<Var, RelId>,
    /// The variables bound to a row a conflict probe returned, whose constructor is any the probe
    /// can reach, and what those rows bind.
    probed: HashSet<Var>,
}

impl<'p, 'a> Walk<'p, 'a> {
    pub(super) fn new(p: &'p Pinned<'a>, map: &'p Map<'a>, src: &'p Terms) -> Self {
        Walk {
            p,
            map,
            src,
            vars: HashMap::new(),
            memo: HashMap::new(),
            rows: HashMap::new(),
            stored: HashMap::new(),
            probed: HashSet::new(),
        }
    }

    /// Note each `let`-bound variable whose value an insert or upsert of the arena stores as a
    /// row.
    #[deny(
        clippy::wildcard_enum_match_arm,
        clippy::match_wildcard_for_single_variants
    )]
    pub(super) fn mark(&mut self) -> R<()> {
        let bound: HashSet<Var> = self
            .src
            .nodes
            .iter()
            .filter_map(|n| match n.kind {
                Kind::Let(v, ..) => Some(v),
                Kind::Var(..)
                | Kind::Const(..)
                | Kind::Prim(..)
                | Kind::Record(..)
                | Kind::Field(..)
                | Kind::Ctor { .. }
                | Kind::Case { .. }
                | Kind::If(..)
                | Kind::Lam(..)
                | Kind::App(..)
                | Kind::Call { .. }
                | Kind::Caller
                | Kind::Key(..)
                | Kind::KeyOf(..)
                | Kind::Deref { .. }
                | Kind::Tag(..)
                | Kind::Try(..)
                | Kind::Rel(..)
                | Kind::Transition(..)
                | Kind::Restrict(..)
                | Kind::EmptySet
                | Kind::Singleton(..)
                | Kind::Union(..)
                | Kind::For(..)
                | Kind::IsEmpty(..)
                | Kind::Exists(..)
                | Kind::Agg { .. }
                | Kind::Group { .. }
                | Kind::Top { .. }
                | Kind::Fix(..)
                | Kind::Fresh(..)
                | Kind::Only(..)
                | Kind::At { .. }
                | Kind::Present { .. }
                | Kind::Reject(..)
                | Kind::Unreachable
                | Kind::Skip
                | Kind::Insert(..)
                | Kind::Delete(..)
                | Kind::Replace(..)
                | Kind::Upsert { .. }
                | Kind::Rejects { .. }
                | Kind::Par(..)
                | Kind::Fail(..)
                | Kind::LetBang(..)
                | Kind::Block(..)
                | Kind::Transact(..) => None,
            })
            .collect();
        for n in &self.src.nodes {
            let (rel, rows) = match &n.kind {
                Kind::Insert(rel, rows) | Kind::Upsert { rel, rows, .. } => (*rel, *rows),
                Kind::Var(..)
                | Kind::Const(..)
                | Kind::Prim(..)
                | Kind::Record(..)
                | Kind::Field(..)
                | Kind::Ctor { .. }
                | Kind::Case { .. }
                | Kind::If(..)
                | Kind::Let(..)
                | Kind::Lam(..)
                | Kind::App(..)
                | Kind::Call { .. }
                | Kind::Caller
                | Kind::Key(..)
                | Kind::KeyOf(..)
                | Kind::Deref { .. }
                | Kind::Tag(..)
                | Kind::Try(..)
                | Kind::Rel(..)
                | Kind::Transition(..)
                | Kind::Restrict(..)
                | Kind::EmptySet
                | Kind::Singleton(..)
                | Kind::Union(..)
                | Kind::For(..)
                | Kind::IsEmpty(..)
                | Kind::Exists(..)
                | Kind::Agg { .. }
                | Kind::Group { .. }
                | Kind::Top { .. }
                | Kind::Fix(..)
                | Kind::Fresh(..)
                | Kind::Only(..)
                | Kind::At { .. }
                | Kind::Present { .. }
                | Kind::Reject(..)
                | Kind::Unreachable
                | Kind::Skip
                | Kind::Delete(..)
                | Kind::Replace(..)
                | Kind::Rejects { .. }
                | Kind::Par(..)
                | Kind::Fail(..)
                | Kind::LetBang(..)
                | Kind::Block(..)
                | Kind::Transact(..) => continue,
            };
            let to = self.relation(rel)?;
            let mut todo = vec![rows];
            while let Some(t) = todo.pop() {
                let kind = self.src.kind(t);
                if let Kind::Var(v) = kind
                    && bound.contains(v)
                {
                    self.stored.insert(*v, to);
                }
                for (c, keeps) in kind.children().into_iter().zip(carries(kind)) {
                    if keeps {
                        todo.push(c);
                    }
                }
            }
        }
        Ok(())
    }

    fn ty(&self, ty: &Ty) -> R<Ty> {
        self.map
            .ty(ty, self.p.hide_new)
            .map_err(|l| self.p.lost(&l, "reaches a value of"))
    }

    /// Returns the variable `v` is at the later version.
    pub(super) fn var(&mut self, out: &mut Terms, v: Var) -> R<Var> {
        if let Some(&x) = self.vars.get(&v) {
            return Ok(x);
        }
        let info = self.src.var_info(v);
        let ty = if self.probed.contains(&v) {
            self.ty_probed(&info.ty)?
        } else {
            self.ty(&info.ty)?
        };
        let x = out.var(&info.name.clone(), ty);
        self.vars.insert(v, x);
        Ok(x)
    }

    fn ty_probed(&self, ty: &Ty) -> R<Ty> {
        self.map
            .ty_unnarrowed(ty)
            .map_err(|l| self.p.lost(&l, "reaches a value of"))
    }

    /// Bind `old` of an upsert into `to` to the constructors its conflict branch may receive.
    #[deny(
        clippy::wildcard_enum_match_arm,
        clippy::match_wildcard_for_single_variants
    )]
    fn bind_probed(&mut self, out: &mut Terms, old: Var, to: RelId, on: &Conflict) -> R<Var> {
        self.probed.insert(old);
        let info = self.src.var_info(old);
        let visible = match &info.ty {
            Ty::Data { def, ctors, .. } if self.p.hide_new => self.map.set(*def, ctors, true),
            Ty::Data { .. }
            | Ty::Base(_)
            | Ty::Param(_)
            | Ty::Unit
            | Ty::Change
            | Ty::Key { .. }
            | Ty::Set(_)
            | Ty::NonEmpty(_)
            | Ty::Group(..)
            | Ty::Tag(_)
            | Ty::Record(_)
            | Ty::Sum { .. }
            | Ty::Fn(..) => None,
        };
        let ty = match self.ty_probed(&info.ty)? {
            Ty::Data { def, args, .. } => Ty::Data {
                def,
                args,
                ctors: {
                    let scope = match on {
                        Conflict::Key => None,
                        Conflict::Unique(i) => {
                            self.map.to.relation(to).indexes[*i as usize].scope.clone()
                        }
                    };
                    match (scope, visible) {
                        (Some(mut scope), Some(visible)) => {
                            scope.retain(|c| visible.contains(c));
                            Some(scope)
                        }
                        (scope, None) => scope,
                        (None, visible) => visible,
                    }
                },
            },
            t @ (Ty::Base(_)
            | Ty::Param(_)
            | Ty::Unit
            | Ty::Change
            | Ty::Key { .. }
            | Ty::Set(_)
            | Ty::NonEmpty(_)
            | Ty::Group(..)
            | Ty::Tag(_)
            | Ty::Record(_)
            | Ty::Sum { .. }
            | Ty::Fn(..)) => t,
        };
        let x = out.var(&info.name.clone(), ty);
        self.vars.insert(old, x);
        Ok(x)
    }

    /// Returns `t` at the later version, `site` the write it is the stored value of.
    pub(super) fn tm(&mut self, out: &mut Terms, t: Tm, site: Option<Site>) -> R<Tm> {
        if let Some(&x) = self.memo.get(&(t, site)) {
            return Ok(x);
        }
        let x = match site {
            Some(s) if self.stored_whole(t, s) => {
                let x = self.tm(out, t, None)?;
                self.rebuilt(out, t, x, s)?
            }
            _ => self.kind(out, t, site)?,
        };
        self.memo.insert((t, site), x);
        Ok(x)
    }

    /// Returns whether `t`, at write site `site`, stores a value it does not build: no
    /// constructor, no branch carrying the site on, and no `let`-bound variable whose value is
    /// built for `site`.
    #[deny(
        clippy::wildcard_enum_match_arm,
        clippy::match_wildcard_for_single_variants
    )]
    fn stored_whole(&self, t: Tm, site: Site) -> bool {
        let kind = self.src.kind(t);
        match kind {
            Kind::Ctor { .. } | Kind::Case { .. } | Kind::Unreachable | Kind::EmptySet => false,
            Kind::Var(v) => {
                !self.rows.contains_key(v)
                    && self
                        .stored
                        .get(v)
                        .is_none_or(|&rel| Site { rel, old: None } != site)
            }
            Kind::Const(..)
            | Kind::Prim(..)
            | Kind::Record(..)
            | Kind::Field(..)
            | Kind::If(..)
            | Kind::Let(..)
            | Kind::Lam(..)
            | Kind::App(..)
            | Kind::Call { .. }
            | Kind::Caller
            | Kind::Key(..)
            | Kind::KeyOf(..)
            | Kind::Deref { .. }
            | Kind::Tag(..)
            | Kind::Try(..)
            | Kind::Rel(..)
            | Kind::Transition(..)
            | Kind::Restrict(..)
            | Kind::Singleton(..)
            | Kind::Union(..)
            | Kind::For(..)
            | Kind::IsEmpty(..)
            | Kind::Exists(..)
            | Kind::Agg { .. }
            | Kind::Group { .. }
            | Kind::Top { .. }
            | Kind::Fix(..)
            | Kind::Fresh(..)
            | Kind::Only(..)
            | Kind::At { .. }
            | Kind::Present { .. }
            | Kind::Reject(..)
            | Kind::Skip
            | Kind::Insert(..)
            | Kind::Delete(..)
            | Kind::Replace(..)
            | Kind::Upsert { .. }
            | Kind::Rejects { .. }
            | Kind::Par(..)
            | Kind::Fail(..)
            | Kind::LetBang(..)
            | Kind::Block(..)
            | Kind::Transact(..) => !carries(kind).contains(&true),
        }
    }

    #[deny(
        clippy::wildcard_enum_match_arm,
        clippy::match_wildcard_for_single_variants
    )]
    fn kind(&mut self, out: &mut Terms, t: Tm, site: Option<Site>) -> R<Tm> {
        let kind = self.src.kind(t).clone();
        let src_ty = self.src.ty(t).clone();
        self.compared(&kind)?;
        let kind = match kind {
            Kind::Var(v) if self.rows.contains_key(&v) => {
                return Err(refused(
                    None,
                    "the fill of a field it writes uses the whole row",
                ));
            }
            Kind::Field(x, name) => return self.field(out, &src_ty, x, &name),
            Kind::Ctor { ctor, fields } => {
                return self.ctor(out, &src_ty, ctor, &fields, site);
            }
            Kind::Case { scrutinee, arms } => {
                return self.case(out, &src_ty, scrutinee, &arms, site);
            }
            Kind::At { .. } => {
                return Err(refused(None, "it reads the past in an `at` block"));
            }
            Kind::Prim(Prim::ReadFile { .. }, _) => {
                return Err(refused(None, "it reads a file"));
            }
            Kind::Insert(rel, rows) => {
                self.written(rel, self.src.ty(rows), None)?;
                let to = self.relation(rel)?;
                let rows = self.tm(out, rows, Some(Site { rel: to, old: None }))?;
                Kind::Insert(to, rows)
            }
            Kind::Replace(rel, rows, f) => return self.replace(out, &src_ty, rel, rows, f),
            Kind::Upsert {
                rel,
                rows,
                on,
                old,
                new,
                otherwise,
                single,
            } => {
                self.written(rel, self.src.ty(rows), None)?;
                let to = self.relation(rel)?;
                let rows = self.tm(out, rows, Some(Site { rel: to, old: None }))?;
                let on = match on {
                    Conflict::Key => Conflict::Key,
                    Conflict::Unique(i) => Conflict::Unique(self.unique(rel, i)?),
                };
                Kind::Upsert {
                    rel: to,
                    rows,
                    on,
                    old: self.bind_probed(out, old, to, &on)?,
                    new: self.var(out, new)?,
                    otherwise: self.tm(out, otherwise, None)?,
                    single,
                }
            }
            Kind::Var(v) if self.probed.contains(&v) => {
                let x = self.var(out, v)?;
                let ty = out.var_info(x).ty.clone();
                return Ok(out.add(Kind::Var(x), ty));
            }
            other @ (Kind::Var(..)
            | Kind::Const(..)
            | Kind::Prim(..)
            | Kind::Record(..)
            | Kind::If(..)
            | Kind::Let(..)
            | Kind::Lam(..)
            | Kind::App(..)
            | Kind::Call { .. }
            | Kind::Caller
            | Kind::Key(..)
            | Kind::KeyOf(..)
            | Kind::Deref { .. }
            | Kind::Tag(..)
            | Kind::Try(..)
            | Kind::Rel(..)
            | Kind::Transition(..)
            | Kind::Restrict(..)
            | Kind::EmptySet
            | Kind::Singleton(..)
            | Kind::Union(..)
            | Kind::For(..)
            | Kind::IsEmpty(..)
            | Kind::Exists(..)
            | Kind::Agg { .. }
            | Kind::Group { .. }
            | Kind::Top { .. }
            | Kind::Fix(..)
            | Kind::Fresh(..)
            | Kind::Only(..)
            | Kind::Present { .. }
            | Kind::Reject(..)
            | Kind::Unreachable
            | Kind::Skip
            | Kind::Delete(..)
            | Kind::Rejects { .. }
            | Kind::Par(..)
            | Kind::Fail(..)
            | Kind::LetBang(..)
            | Kind::Block(..)
            | Kind::Transact(..)) => {
                if let Kind::Let(v, value, _) = &other
                    && forms::reads_probed(self.src, &self.probed, *value)
                {
                    self.probed.insert(*v);
                }
                self.generic(out, &other, site)?
            }
        };
        let ty = self.ty(&src_ty)?;
        Ok(out.add(kind, ty))
    }

    /// Returns `kind` with its children, variables and named objects at the later version.
    #[deny(
        clippy::wildcard_enum_match_arm,
        clippy::match_wildcard_for_single_variants
    )]
    fn generic(&mut self, out: &mut Terms, kind: &Kind, site: Option<Site>) -> R<Kind> {
        let carried = carries(kind);
        let bound = if let Kind::Let(v, ..) = kind {
            self.stored.get(v).map(|&rel| Site { rel, old: None })
        } else {
            None
        };
        let mut children = Vec::new();
        for (i, (c, keeps)) in kind.children().into_iter().zip(carried).enumerate() {
            let at = if i == 0 && bound.is_some() {
                bound
            } else {
                site.filter(|_| keeps)
            };
            children.push(self.tm(out, c, at)?);
        }
        let mut bound = Vec::new();
        kind.with_vars(&mut |v| {
            bound.push(v);
            v
        });
        for v in bound {
            self.var(out, v)?;
        }
        let kind = kind
            .with_children(&children)
            .with_vars(&mut |v| self.vars[&v]);
        Ok(match kind {
            Kind::Rel(r, set) => {
                let def = self.map.from.relation(r).elem_def;
                Kind::Rel(self.relation(r)?, self.map.set(def, &set, true))
            }
            Kind::Transition(tr, r, set) => {
                let def = self.map.from.relation(r).elem_def;
                Kind::Transition(tr, self.relation(r)?, self.map.set(def, &set, false))
            }
            Kind::Restrict(x, set) => {
                let Ty::Data { def, .. } = elem(out.ty(x)).clone() else {
                    return Ok(Kind::Restrict(x, set));
                };
                let from = self.earlier_def(def);
                let set = self
                    .map
                    .set(from, &Some(set), false)
                    .unwrap_or_else(|| (0..self.map.to.ty(def).ctors().len() as u32).collect());
                Kind::Restrict(x, set)
            }
            Kind::KeyOf(r, x) => Kind::KeyOf(self.relation(r)?, x),
            Kind::Fresh(g, r) => Kind::Fresh(g, self.relation(r)?),
            Kind::Delete(r, x) => Kind::Delete(self.relation(r)?, x),
            Kind::Rejects { into, insert } => Kind::Rejects {
                into: self.relation(into)?,
                insert,
            },
            Kind::Call {
                func,
                type_args,
                args,
            } => Kind::Call {
                func: self.p.function(func).ok_or_else(|| {
                    refused(
                        self.p
                            .blame_function(self.map.from.ids.functions.get(&func).copied()),
                        format!(
                            "it calls `{}`, whose code at the pin is not the current one",
                            self.map.from.function(func).name
                        ),
                    )
                })?,
                type_args: type_args
                    .iter()
                    .map(|t| self.ty(t))
                    .collect::<R<Vec<_>>>()?,
                args,
            },
            Kind::Agg {
                op: Agg::User(a),
                var,
                source,
                body,
            } => Kind::Agg {
                op: Agg::User(self.p.aggregate(a).ok_or_else(|| {
                    refused(
                        self.p.blame_function(
                            self.map.from.ids.aggregates.get(&(a as usize)).copied(),
                        ),
                        "it folds by an aggregate whose code at the pin is not the current one",
                    )
                })?),
                var,
                source,
                body,
            },
            same @ (Kind::Var(..)
            | Kind::Const(..)
            | Kind::Prim(..)
            | Kind::Record(..)
            | Kind::Field(..)
            | Kind::Ctor { .. }
            | Kind::Case { .. }
            | Kind::If(..)
            | Kind::Let(..)
            | Kind::Lam(..)
            | Kind::App(..)
            | Kind::Caller
            | Kind::Key(..)
            | Kind::Deref { .. }
            | Kind::Tag(..)
            | Kind::Try(..)
            | Kind::EmptySet
            | Kind::Singleton(..)
            | Kind::Union(..)
            | Kind::For(..)
            | Kind::IsEmpty(..)
            | Kind::Exists(..)
            | Kind::Agg { .. }
            | Kind::Group { .. }
            | Kind::Top { .. }
            | Kind::Fix(..)
            | Kind::Only(..)
            | Kind::At { .. }
            | Kind::Present { .. }
            | Kind::Reject(..)
            | Kind::Unreachable
            | Kind::Skip
            | Kind::Insert(..)
            | Kind::Replace(..)
            | Kind::Upsert { .. }
            | Kind::Par(..)
            | Kind::Fail(..)
            | Kind::LetBang(..)
            | Kind::Block(..)
            | Kind::Transact(..)) => same,
        })
    }

    /// Returns the type of the pin that `def`, a type of the later version, is.
    fn earlier_def(&self, def: TypeId) -> TypeId {
        (0..self.map.from.types.len() as u32)
            .map(TypeId)
            .find(|&d| self.map.def(d) == Some(def))
            .unwrap_or(def)
    }

    fn relation(&self, r: RelId) -> R<RelId> {
        self.map.relation(r).ok_or_else(|| self.p.gone_relation(r))
    }

    /// Returns the position at the later version of the unique at position `i` of `rel`.
    fn unique(&self, rel: RelId, i: u32) -> R<u32> {
        let to = self.relation(rel)?;
        let from = self.map.from.ids.indexes.get(&(rel, i as usize)).copied();
        from.and_then(|id| {
            self.map
                .to
                .ids
                .indexes
                .iter()
                .find(|((r, _), j)| *r == to && **j == id)
        })
        .map(|((_, j), _)| *j as u32)
        .ok_or_else(|| {
            refused(
                self.p.blame(
                    |o| matches!(o.op, Resolved::DropUnique { index } if Some(index) == from),
                ),
                "its conflict target is dropped",
            )
        })
    }

    /// Refuse a comparison, grouping, ordering or union of values that do not carry whole, and a
    /// count, fold, `top`, `take` or `only` of a set whose elements may merge at the pin.
    #[deny(
        clippy::wildcard_enum_match_arm,
        clippy::match_wildcard_for_single_variants
    )]
    fn compared(&self, kind: &Kind) -> R<()> {
        let check = |t: Tm, ordered: bool| {
            self.map
                .whole(self.src.ty(t), true, ordered)
                .map_err(|l| self.p.lost(&l, "compares"))
        };
        match kind {
            Kind::Prim(Prim::Eq | Prim::Ne, xs) => xs.iter().try_for_each(|&x| check(x, false)),
            Kind::Prim(Prim::Lt | Prim::Le | Prim::Gt | Prim::Ge, xs) => {
                xs.iter().try_for_each(|&x| check(x, true))
            }
            Kind::Group { by, .. } => check(*by, false),
            Kind::Union(a, _) => check(*a, false),
            Kind::Top { keys, source, .. } => {
                self.distinct(*source)?;
                keys.iter().try_for_each(|&(k, _)| check(k, true))
            }
            Kind::Present {
                keys, source, take, ..
            } => {
                if take.is_some() {
                    self.distinct(*source)?;
                }
                keys.iter().try_for_each(|&(k, _)| check(k, true))
            }
            Kind::Agg {
                op, body, source, ..
            } => {
                self.distinct(*source)?;
                match op {
                    Agg::Min | Agg::Max => check(*body, true),
                    Agg::Count | Agg::Sum | Agg::Avg | Agg::User(_) => Ok(()),
                }
            }
            Kind::Only(x) => self.distinct(*x),
            Kind::Call { args, .. } => args.iter().try_for_each(|&x| check(x, false)),
            Kind::Var(..)
            | Kind::Const(..)
            | Kind::Prim(..)
            | Kind::Record(..)
            | Kind::Field(..)
            | Kind::Ctor { .. }
            | Kind::Case { .. }
            | Kind::If(..)
            | Kind::Let(..)
            | Kind::Lam(..)
            | Kind::App(..)
            | Kind::Caller
            | Kind::Key(..)
            | Kind::KeyOf(..)
            | Kind::Deref { .. }
            | Kind::Tag(..)
            | Kind::Try(..)
            | Kind::Rel(..)
            | Kind::Transition(..)
            | Kind::Restrict(..)
            | Kind::EmptySet
            | Kind::Singleton(..)
            | Kind::For(..)
            | Kind::IsEmpty(..)
            | Kind::Exists(..)
            | Kind::Fix(..)
            | Kind::Fresh(..)
            | Kind::At { .. }
            | Kind::Reject(..)
            | Kind::Unreachable
            | Kind::Skip
            | Kind::Insert(..)
            | Kind::Delete(..)
            | Kind::Replace(..)
            | Kind::Upsert { .. }
            | Kind::Rejects { .. }
            | Kind::Par(..)
            | Kind::Fail(..)
            | Kind::LetBang(..)
            | Kind::Block(..)
            | Kind::Transact(..) => Ok(()),
        }
    }

    /// Refuse `set` when two of its elements may be one value at the pin: its elements do not
    /// carry whole, and are no rows read from one relation, whose keys keep them apart.
    fn distinct(&self, set: Tm) -> R<()> {
        if forms::rows_of(self.src, set) {
            return Ok(());
        }
        self.map
            .whole(elem(self.src.ty(set)), true, false)
            .map_err(|l| self.p.lost(&l, "counts the values of"))
    }

    /// Refuse a write into `rel` of a constructor of `rows`, a set of rows, that is enabled at
    /// the pin and not later; `kept`, the constructors of the rows a replace replaces, are not
    /// written into.
    fn written(&self, rel: RelId, rows: &Ty, kept: Option<&[u32]>) -> R<()> {
        let elem = match rows {
            Ty::Set(t) | Ty::NonEmpty(t) => t.as_ref(),
            t => t,
        };
        let Some(set) = Map::ctor_set(self.map.from, elem) else {
            return Ok(());
        };
        for c in set {
            if kept.is_some_and(|k| k.contains(&c)) {
                continue;
            }
            self.p.writable(rel, c)?;
        }
        Ok(())
    }

    fn field(&mut self, out: &mut Terms, src_ty: &Ty, x: Tm, name: &str) -> R<Tm> {
        let xt = self.src.ty(x).clone();
        let Ty::Data { def, .. } = &xt else {
            let x2 = self.tm(out, x, None)?;
            let fty = self.ty(src_ty)?;
            return Ok(out.add(Kind::Field(x2, name.to_string()), fty));
        };
        let set = Map::ctor_set(self.map.from, &xt).unwrap_or_default();
        let kept = |&c: &u32| self.map.ctor(*def, c).is_some();
        if !set.iter().any(kept) {
            let x2 = self.tm(out, x, None)?;
            let fty = self.ty(src_ty)?;
            return Ok(out.add(Kind::Field(x2, name.to_string()), fty));
        }
        let given = match self.src.kind(x) {
            Kind::Var(v) => self.rows.get(v).cloned(),
            _ => None,
        };
        let mut images = BTreeMap::new();
        for c in set {
            let Some(k) = self.map.ctor(*def, c) else {
                continue;
            };
            if given.as_ref().is_some_and(|g| g.ctor != k) {
                continue;
            }
            let Some(image) = self.map.field(*def, Some(c), name) else {
                let id = self
                    .map
                    .from
                    .ids
                    .fields
                    .get(&super::map::slot(self.map.from, *def, c, name))
                    .copied();
                return Err(refused(
                    self.p.blame_field(id, false),
                    format!(
                        "it reads field `{name}` of `{}`, which the current version lacks",
                        self.map.from.ty(*def).name
                    ),
                ));
            };
            images.insert(k, image);
        }
        let image = images
            .values()
            .next()
            .expect("a field has a surviving owner");
        if let Some(g) = given {
            return g.fields.get(image).copied().ok_or_else(|| {
                refused(
                    None,
                    format!(
                        "the fill of a field it writes reads `{image}`, which it does not give"
                    ),
                )
            });
        }
        let x2 = self.tm(out, x, None)?;
        let fty = if forms::reads_probed(self.src, &self.probed, x) {
            self.ty_probed(src_ty)?
        } else {
            self.ty(src_ty)?
        };
        if images.values().all(|n| n == image) {
            return Ok(out.add(Kind::Field(x2, image.clone()), fty));
        }
        let xt = out.ty(x2).clone();
        let mut arms = Vec::new();
        for k in Map::ctor_set(self.map.to, &xt).unwrap_or_default() {
            let arm_ty = restricted(self.map.to, &xt, k);
            let y = out.var("field", arm_ty.clone());
            let body = match images.get(&k) {
                Some(image) => {
                    let row = out.add(Kind::Var(y), arm_ty);
                    out.add(Kind::Field(row, image.clone()), fty.clone())
                }
                None => out.add(Kind::Unreachable, fty.clone()),
            };
            arms.push((k, y, body));
        }
        Ok(out.add(
            Kind::Case {
                scrutinee: x2,
                arms,
            },
            fty,
        ))
    }

    fn ctor(
        &mut self,
        out: &mut Terms,
        src_ty: &Ty,
        ctor: u32,
        fields: &[(String, Tm)],
        site: Option<Site>,
    ) -> R<Tm> {
        let ty = self.ty(src_ty)?;
        if let Ty::Tag(t) = src_ty
            && let Ty::Data { def, .. } = t.as_ref()
        {
            let k = self.built(*def, ctor)?;
            let fields = Vec::new();
            return Ok(out.add(Kind::Ctor { ctor: k, fields }, ty));
        }
        let Ty::Data { def, .. } = src_ty else {
            let mut given = Vec::with_capacity(fields.len());
            for (n, f) in fields {
                given.push((n.clone(), self.tm(out, *f, None)?));
            }
            return Ok(out.add(
                Kind::Ctor {
                    ctor,
                    fields: given,
                },
                ty,
            ));
        };
        let Ty::Data {
            def: to,
            args: to_args,
            ..
        } = &ty
        else {
            unreachable!("a declared type maps to a declared type");
        };
        let k = self.built(*def, ctor)?;
        let mut given = BTreeMap::new();
        for (n, f) in fields {
            if let Some(image) = self.map.field(*def, Some(ctor), n) {
                given.insert(image, self.tm(out, *f, None)?);
            }
        }
        for (n, fty) in Map::fields(self.map.to, *to, to_args, k) {
            if given.contains_key(&n) {
                continue;
            }
            let value = match site.filter(|s| self.map.to.relation(s.rel).elem_def == *to) {
                Some(s) => {
                    let lacked = Lacked {
                        def: *to,
                        k,
                        name: &n,
                        ty: &fty,
                    };
                    self.missing(out, s, &lacked, &given)?
                }
                None => {
                    let id = self
                        .map
                        .to
                        .ids
                        .fields
                        .get(&super::map::slot(self.map.to, *to, k, &n))
                        .copied();
                    return Err(refused(
                        self.p.blame_field(id, true),
                        format!(
                            "it builds `{}` without field `{n}`, outside a write",
                            self.p.ctor_name(*def, ctor)
                        ),
                    ));
                }
            };
            given.insert(n, value);
        }
        let fields = given.into_iter().collect();
        Ok(out.add(Kind::Ctor { ctor: k, fields }, ty))
    }

    /// Returns the later position of constructor `ctor` of `def`, a constructor a term builds.
    fn built(&self, def: TypeId, ctor: u32) -> R<u32> {
        self.map.ctor(def, ctor).ok_or_else(|| {
            refused(
                self.p.blame_ctor(def, ctor),
                format!(
                    "it builds `{}`, which the current version lacks",
                    self.p.ctor_name(def, ctor)
                ),
            )
        })
    }

    /// Returns the value of field `lacked` that a write at `site` of a value with the fields
    /// `given` lacks: the replaced row's when it has the field's constructor, else the
    /// relation's default, else the fill of the operator that added the field.
    fn missing(
        &mut self,
        out: &mut Terms,
        site: Site,
        lacked: &Lacked<'_>,
        given: &BTreeMap<String, Tm>,
    ) -> R<Tm> {
        let fallback = self.fallback(out, site.rel, lacked, given)?;
        let Some(old) = site.old else {
            return Ok(fallback);
        };
        let (def, k, name, fty) = (lacked.def, lacked.k, lacked.name, lacked.ty);
        let old_ty = out.var_info(old).ty.clone();
        let set = Map::ctor_set(self.map.to, &old_ty).unwrap_or_default();
        if !set.contains(&k) {
            return Ok(fallback);
        }
        let row = out.add(Kind::Var(old), old_ty.clone());
        if set == [k] {
            return Ok(out.add(Kind::Field(row, name.to_string()), fty.clone()));
        }
        let Ty::Data { args, .. } = &old_ty else {
            return Ok(fallback);
        };
        let mut arms = Vec::with_capacity(set.len());
        for c in set {
            let arm_ty = Ty::Data {
                def,
                args: args.clone(),
                ctors: self.map.to.normalize_ctors(def, vec![c]),
            };
            let y = out.var(&format!("{}_{c}", out.var_info(old).name), arm_ty.clone());
            let body = if c == k {
                let y_tm = out.add(Kind::Var(y), arm_ty);
                out.add(Kind::Field(y_tm, name.to_string()), fty.clone())
            } else {
                fallback
            };
            arms.push((c, y, body));
        }
        Ok(out.add(
            Kind::Case {
                scrutinee: row,
                arms,
            },
            fty.clone(),
        ))
    }

    /// Returns the value of field `lacked` that a write into `rel` of a value with the fields
    /// `given` lacks when no replaced row gives it: the relation's default, else the fill of the
    /// operator that added the field.
    fn fallback(
        &mut self,
        out: &mut Terms,
        rel: RelId,
        lacked: &Lacked<'_>,
        given: &BTreeMap<String, Tm>,
    ) -> R<Tm> {
        let (def, k, name, fty) = (lacked.def, lacked.k, lacked.name, lacked.ty);
        let r = self.map.to.relation(rel);
        let default = r
            .defaults
            .iter()
            .position(|d| d.field == name && d.ctor == Some(k))
            .or_else(|| {
                r.defaults
                    .iter()
                    .position(|d| d.field == name && d.ctor.is_none())
            });
        if let Some(i) = default {
            return match &r.defaults[i].value {
                DefaultValue::Generate(g) => Ok(out.add(Kind::Fresh(*g, rel), fty.clone())),
                DefaultValue::Expr(_) => {
                    let normal = self.p.normal()?;
                    let Some(&t) = normal.defaults.get(&(rel, i)) else {
                        return Err(refused(
                            None,
                            format!("the default of `{name}` has no term"),
                        ));
                    };
                    let same = Map::new(self.map.to, self.map.to);
                    Walk::new(self.p, &same, &normal.terms).tm(out, t, None)
                }
            };
        }
        let id = self
            .map
            .to
            .ids
            .fields
            .get(&super::map::slot(self.map.to, def, k, name))
            .copied();
        let Some((fill, op)) = self.p.fill(def, k, name)? else {
            return Err(refused(
                self.p.blame_field(id, true),
                format!("it writes no `{name}`, which has no default and no fill"),
            ));
        };
        let blamed = |u: super::Untranslatable| super::Untranslatable {
            operator: u.operator.or_else(|| Some(op.clone())),
            ..u
        };
        let map = Map::new(&fill.schema, self.map.to);
        let mut w = Walk::new(self.p, &map, &fill.query.terms);
        let [row] = fill.query.params.as_slice() else {
            return Err(refused(Some(op.clone()), "a fill is a function of one row"));
        };
        w.rows.insert(
            *row,
            Rc::new(Given {
                ctor: k,
                fields: given.clone(),
            }),
        );
        w.tm(out, fill.query.body, None).map_err(blamed)
    }

    /// Returns `x`, the value at the later version of `t`, a value or set of values of `ty`
    /// stored whole at `site`, rebuilt field by field so that a field its constructor gained
    /// since the pin takes its value as a field the write lacks.
    fn rebuilt(&mut self, out: &mut Terms, t: Tm, x: Tm, site: Site) -> R<Tm> {
        let ty = self.src.ty(t).clone();
        if let Ty::Set(e) | Ty::NonEmpty(e) = &ty {
            if !self.gains(e, site) {
                return Ok(x);
            }
            let to_elem = self.ty(e)?;
            let v = out.var("row", to_elem.clone());
            let row = out.add(Kind::Var(v), to_elem);
            let one = self.whole_row(out, e, row, site)?;
            let body = out.add(Kind::Singleton(one), out.ty(x).clone());
            return Ok(out.add(Kind::For(v, x, body), out.ty(x).clone()));
        }
        if !self.gains(&ty, site) {
            return Ok(x);
        }
        self.whole_row(out, &ty, x, site)
    }

    /// Returns whether `ty` is the element type of the relation `site` writes and one of its
    /// constructors gained a field since the pin.
    fn gains(&self, ty: &Ty, site: Site) -> bool {
        let Ty::Data { def, .. } = ty else {
            return false;
        };
        if self.map.def(*def) != Some(self.map.to.relation(site.rel).elem_def) {
            return false;
        }
        let names = |schema: &crate::schema::Schema, def: TypeId, c: u32| -> Vec<String> {
            let t = schema.ty(def);
            t.header()
                .iter()
                .chain(
                    t.ctors()
                        .get(c as usize)
                        .into_iter()
                        .flat_map(|k| &k.fields),
                )
                .map(|f| f.name.clone())
                .collect()
        };
        let to = self.map.to.relation(site.rel).elem_def;
        Map::ctor_set(self.map.from, ty)
            .unwrap_or_default()
            .into_iter()
            .any(|c| {
                let Some(k) = self.map.ctor(*def, c) else {
                    return false;
                };
                let images: Vec<String> = names(self.map.from, *def, c)
                    .iter()
                    .filter_map(|f| self.map.field(*def, Some(c), f))
                    .collect();
                names(self.map.to, to, k)
                    .iter()
                    .any(|n| !images.contains(n))
            })
    }

    /// Returns `x`, a value of `ty` at the later version, as a match on its constructor whose
    /// every arm rebuilds the value from its fields of the pin and the fields it lacks.
    fn whole_row(&mut self, out: &mut Terms, ty: &Ty, x: Tm, site: Site) -> R<Tm> {
        let Ty::Data { def, args, .. } = ty else {
            unreachable!("a stored row is of a declared type");
        };
        let to_ty = out.ty(x).clone();
        let Ty::Data {
            def: to,
            args: to_args,
            ..
        } = &to_ty
        else {
            unreachable!("a declared type maps to a declared type");
        };
        let mut arms = Vec::new();
        for k in Map::ctor_set(self.map.to, &to_ty).unwrap_or_default() {
            let arm_ty = restricted(self.map.to, &to_ty, k);
            let y = out.var("stored", arm_ty.clone());
            let Some(c) = self.map.ctor_back(*def, k) else {
                arms.push((k, y, out.add(Kind::Unreachable, to_ty.clone())));
                continue;
            };
            let y_tm = out.add(Kind::Var(y), arm_ty);
            let mut given = BTreeMap::new();
            for (f, _) in Map::fields(self.map.from, *def, args, c) {
                let Some(image) = self.map.field(*def, Some(c), &f) else {
                    continue;
                };
                let fty = Map::fields(self.map.to, *to, to_args, k)
                    .into_iter()
                    .find(|(n, _)| *n == image)
                    .map(|(_, t)| t)
                    .expect("a field's image is a field of its constructor's image");
                given.insert(image.clone(), out.add(Kind::Field(y_tm, image), fty));
            }
            for (n, fty) in Map::fields(self.map.to, *to, to_args, k) {
                if given.contains_key(&n) {
                    continue;
                }
                let lacked = Lacked {
                    def: *to,
                    k,
                    name: &n,
                    ty: &fty,
                };
                let value = self.missing(out, site, &lacked, &given)?;
                given.insert(n, value);
            }
            let fields = given.into_iter().collect();
            arms.push((k, y, out.add(Kind::Ctor { ctor: k, fields }, to_ty.clone())));
        }
        Ok(out.add(Kind::Case { scrutinee: x, arms }, to_ty))
    }

    fn case(
        &mut self,
        out: &mut Terms,
        src_ty: &Ty,
        scrutinee: Tm,
        arms: &[(u32, Var, Tm)],
        site: Option<Site>,
    ) -> R<Tm> {
        let sty = self.src.ty(scrutinee).clone();
        let def = match &sty {
            Ty::Data { def, .. } => Some(*def),
            Ty::Tag(t) => match t.as_ref() {
                Ty::Data { def, .. } => Some(*def),
                _ => None,
            },
            _ => None,
        };
        if let Kind::Var(v) = self.src.kind(scrutinee)
            && let Some(g) = self.rows.get(v).cloned()
            && let Some(def) = def
        {
            let arm = arms
                .iter()
                .find(|(c, _, _)| self.map.ctor(def, *c) == Some(g.ctor));
            let Some(&(_, y, body)) = arm else {
                return Err(refused(
                    None,
                    "the fill of a field it writes has no arm for its constructor",
                ));
            };
            self.rows.insert(y, g);
            return self.tm(out, body, site);
        }
        let s = self.tm(out, scrutinee, None)?;
        let ty = self.ty(src_ty)?;
        let Some(def) = def else {
            let kind = self.generic(
                out,
                &Kind::Case {
                    scrutinee,
                    arms: arms.to_vec(),
                },
                site,
            )?;
            return Ok(out.add(kind, ty));
        };
        let mut by_ctor = BTreeMap::new();
        for &(c, y, body) in arms {
            let Some(k) = self.map.ctor(def, c) else {
                continue;
            };
            if forms::reads_probed(self.src, &self.probed, scrutinee) {
                self.probed.insert(y);
            }
            let y = self.var(out, y)?;
            by_ctor.insert(k, (y, self.tm(out, body, site)?));
        }
        let s_ty = out.ty(s).clone();
        let set = Map::ctor_set(self.map.to, &s_ty).unwrap_or_default();
        let mut new_arms = Vec::with_capacity(set.len());
        for k in set {
            let (y, body) = match by_ctor.remove(&k) {
                Some(arm) => arm,
                None => {
                    let arm_ty = restricted(self.map.to, &s_ty, k);
                    let y = out.var("unreachable", arm_ty);
                    (y, out.add(Kind::Unreachable, ty.clone()))
                }
            };
            new_arms.push((k, y, body));
        }
        Ok(out.add(
            Kind::Case {
                scrutinee: s,
                arms: new_arms,
            },
            ty,
        ))
    }

    fn replace(&mut self, out: &mut Terms, src_ty: &Ty, rel: RelId, rows: Tm, f: Tm) -> R<Tm> {
        let kept = Map::ctor_set(self.map.from, elem(self.src.ty(rows)));
        let to = self.relation(rel)?;
        let rows2 = self.tm(out, rows, None)?;
        let f2 = match self.src.kind(f).clone() {
            Kind::Lam(x, body) => {
                self.written(rel, self.src.ty(body), kept.as_deref())?;
                if let Some(row) = forms::probed_rows(self.src, &self.probed, rows) {
                    let ty = out.var_info(self.vars[&row]).ty.clone();
                    self.probed.insert(x);
                    let x2 = out.var(&self.src.var_info(x).name.clone(), ty);
                    self.vars.insert(x, x2);
                }
                let x2 = self.var(out, x)?;
                let body2 = self.tm(
                    out,
                    body,
                    Some(Site {
                        rel: to,
                        old: Some(x2),
                    }),
                )?;
                let fty = self.ty(self.src.ty(f))?;
                out.add(Kind::Lam(x2, body2), fty)
            }
            _ => self.tm(out, f, None)?,
        };
        let ty = self.ty(src_ty)?;
        Ok(out.add(Kind::Replace(to, rows2, f2), ty))
    }
}

fn elem(ty: &Ty) -> &Ty {
    match ty {
        Ty::Set(t) | Ty::NonEmpty(t) => t,
        t => t,
    }
}

/// Returns `ty`, a declared sum or its tags, restricted to constructor `k`.
fn restricted(schema: &crate::schema::Schema, ty: &Ty, k: u32) -> Ty {
    match ty {
        Ty::Data { def, args, .. } => Ty::Data {
            def: *def,
            args: args.clone(),
            ctors: schema.normalize_ctors(*def, vec![k]),
        },
        Ty::Tag(t) => Ty::Tag(Box::new(restricted(schema, t, k))),
        t => t.clone(),
    }
}
