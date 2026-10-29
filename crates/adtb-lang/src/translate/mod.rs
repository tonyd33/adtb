//! Translation of a callable's normal form at a pinned schema version to a later version: every
//! relation, type, constructor and field mapped by catalog id, a write's missing fields filled.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::rc::Rc;

use crate::check::Program;
use crate::evolve::{Clause, Code, Resolved, RowFn, RowMap, Source};
use crate::kernel::{Kind, Terms, Tm};
use crate::norm::{self, NormalProgram, NormalQuery};
use crate::schema::{Schema, Stage};
use crate::types::*;

mod map;
#[cfg(test)]
mod tests;
mod walk;

pub use map::{Loss, Map};

/// Why a callable has no translation: the operator it fails across, when one is to blame, and
/// the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Untranslatable {
    pub operator: Option<String>,
    pub reason: String,
}

impl fmt::Display for Untranslatable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.operator {
            Some(op) => write!(f, "across `{op}`: {}", self.reason),
            None => f.write_str(&self.reason),
        }
    }
}

/// One operator logged after the pin.
#[derive(Debug)]
struct Op {
    /// The migration it belongs to and its position among that migration's operators.
    at: (usize, usize),
    text: String,
    op: Resolved,
    clauses: Vec<Clause>,
}

/// The translation from a pinned version to a later one.
#[derive(Debug)]
pub struct Pinned<'a> {
    map: Map<'a>,
    from_source: &'a Source,
    to_source: &'a Source,
    to_program: &'a Program,
    hide_new: bool,
    ops: Vec<Op>,
    normal: RefCell<Option<Rc<NormalProgram>>>,
}

type R<T> = Result<T, Untranslatable>;

fn refused(operator: Option<String>, reason: impl Into<String>) -> Untranslatable {
    Untranslatable {
        operator,
        reason: reason.into(),
    }
}

impl<'a> Pinned<'a> {
    /// Returns the translation from `from`, the schema of `from_source` with any functions of
    /// the bundle, to `to`, the schema of `to_source` checked as `to_program`, a later version
    /// of the same log. A bundle that declares `hide new constructors` is `hide_new`.
    pub fn new(
        from: &'a Schema,
        from_source: &'a Source,
        to: &'a Schema,
        to_source: &'a Source,
        to_program: &'a Program,
        hide_new: bool,
    ) -> R<Pinned<'a>> {
        let mut ops = Vec::new();
        for (i, logged) in to_source.log.iter().enumerate() {
            if logged.number <= from_source.version {
                continue;
            }
            let resolved = to_source.resolved(logged.number).map_err(|ds| {
                let first = logged
                    .operators
                    .first()
                    .map(|op| operator_text(&logged.text, op));
                refused(
                    first,
                    ds.into_iter()
                        .map(|d| d.message)
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            })?;
            for (k, (op, r)) in logged.operators.iter().zip(resolved).enumerate() {
                let text = operator_text(&logged.text, op);
                ops.push(Op {
                    at: (i, k),
                    text,
                    op: r.op,
                    clauses: r.clauses,
                });
            }
        }
        Ok(Pinned {
            map: Map::new(from, to),
            from_source,
            to_source,
            to_program,
            hide_new,
            ops,
            normal: RefCell::new(None),
        })
    }

    pub fn map(&self) -> &Map<'a> {
        &self.map
    }

    /// Returns the translation of `q`, the normal form of a query or transaction at the pinned
    /// version.
    ///
    /// Fail when it reads a field or constructor the later version lost, or when a value it
    /// returns, takes, compares, orders or passes to a function does not carry whole; when a
    /// constructor it writes is enabled at the pin and not later; when an operator it meets is
    /// one this translation does not cross.
    pub fn query(&self, q: &NormalQuery) -> R<NormalQuery> {
        self.uncrossed(&q.terms)?;
        let mut out = Terms::default();
        let mut w = walk::Walk::new(self, &self.map, &q.terms);
        w.mark()?;
        let mut params = Vec::with_capacity(q.params.len());
        for &p in &q.params {
            let ty = &q.terms.var_info(p).ty;
            self.map
                .whole(ty, true, false)
                .map_err(|l| self.lost(&l, "takes a parameter of"))?;
            params.push(w.var(&mut out, p)?);
        }
        self.map
            .whole(q.terms.ty(q.body), false, false)
            .map_err(|l| self.lost(&l, "returns"))?;
        let body = w.tm(&mut out, q.body, None)?;
        Ok(NormalQuery {
            terms: out,
            params,
            body,
        })
    }

    /// Returns the refusal of a value that `what` a type that does not carry whole.
    fn lost(&self, l: &Loss, what: &str) -> Untranslatable {
        let from = self.map.from;
        match l {
            Loss::Type(def) => {
                let id = from.ids.types.get(def).copied();
                refused(
                    self.blame(|o| matches!(o.op, Resolved::DropType { sum } if Some(sum) == id)),
                    format!(
                        "it {what} `{}`, which the current version lacks",
                        from.ty(*def).name
                    ),
                )
            }
            Loss::Relation(rel) => self.gone_relation(*rel),
            Loss::Ctor(def, c) => refused(
                self.blame_ctor(*def, *c),
                format!(
                    "it {what} `{}`, which the current version lacks",
                    self.ctor_name(*def, *c)
                ),
            ),
            Loss::Field {
                ty,
                field,
                id,
                added,
            } => refused(
                self.blame_field(*id, *added),
                format!("it {what} `{ty}` whole, and its field `{field}` differs"),
            ),
            Loss::Order(def) => refused(
                self.blame_order(*def),
                format!(
                    "it {what} `{}` ordered, and its order differs",
                    from.ty(*def).name
                ),
            ),
        }
    }

    fn gone_relation(&self, rel: RelId) -> Untranslatable {
        let id = self.map.from.ids.relations.get(&rel).copied();
        refused(
            self.blame(
                |o| matches!(o.op, Resolved::DropRelation { relation } if Some(relation) == id),
            ),
            format!(
                "it reaches `{}`, which the current version lacks",
                self.map.from.relation(rel).name
            ),
        )
    }

    /// Returns the text of the first operator `p` holds of, else of a revert since the pin, else
    /// of the first operator of the last migration since the pin.
    fn blame(&self, p: impl Fn(&Op) -> bool) -> Option<String> {
        let last = self.ops.last().map(|o| o.at.0);
        self.ops
            .iter()
            .find(|o| p(o))
            .or_else(|| {
                self.ops
                    .iter()
                    .find(|o| matches!(o.op, Resolved::Revert { .. }))
            })
            .or_else(|| self.ops.iter().find(|o| Some(o.at.0) == last))
            .map(|o| o.text.clone())
    }

    /// Returns the text of the first operator that adds a constructor to `def`, or that adds,
    /// drops or moves a field of `def`.
    fn blame_order(&self, def: TypeId) -> Option<String> {
        let from = self.map.from;
        let id = from.ids.types.get(&def).copied();
        let to = self.map.def(def);
        let of = |field: u64| {
            from.ids
                .fields
                .iter()
                .any(|((d, _, _), &i)| *d == def && i == field)
                || self
                    .map
                    .to
                    .ids
                    .fields
                    .iter()
                    .any(|((d, _, _), &i)| Some(*d) == to && i == field)
        };
        self.blame(|o| match &o.op {
            Resolved::AddConstructor { sum, .. } => Some(*sum) == id,
            Resolved::AddField { field, .. }
            | Resolved::DropField { field }
            | Resolved::MoveField { field, .. } => of(*field),
            _ => false,
        })
    }

    /// Returns the text of the first operator that redefines or drops function or aggregate
    /// `id`.
    fn blame_function(&self, id: Option<u64>) -> Option<String> {
        self.blame(|o| match &o.op {
            Resolved::RedefineFn { function, .. }
            | Resolved::DropFn { function }
            | Resolved::DropAggregate { function }
            | Resolved::DefineAggregate { function, .. } => Some(*function) == id,
            _ => false,
        })
    }

    /// Returns the text of the first operator that removes, splits, unnests, merges or nests
    /// constructor `c` of `def`.
    fn blame_ctor(&self, def: TypeId, c: u32) -> Option<String> {
        let id = self.map.from.ids.ctors.get(&(def, c)).copied();
        self.blame(|o| match &o.op {
            Resolved::RemoveConstructor { ctor }
            | Resolved::Split { ctor, .. }
            | Resolved::Unnest { ctor } => Some(*ctor) == id,
            Resolved::Merge { .. } | Resolved::Nest { .. } => o
                .clauses
                .iter()
                .any(|c| matches!(c, Clause::Member { ctor } if Some(*ctor) == id)),
            _ => false,
        })
    }

    fn blame_field(&self, id: Option<u64>, added: bool) -> Option<String> {
        self.blame(|o| {
            let named = match &o.op {
                Resolved::AddField { field, .. } if added => Some(*field),
                Resolved::DropField { field } if !added => Some(*field),
                Resolved::MoveField { field, .. } | Resolved::AlterField { field, .. } => {
                    Some(*field)
                }
                _ => None,
            };
            named.is_some() && named == id
        })
    }

    /// Refuse a callable that meets an operator this translation does not cross: a split,
    /// merge, nest or unnest of a type it reaches, a move or alter of a field of one, a rekey of
    /// a relation it reaches.
    fn uncrossed(&self, terms: &Terms) -> R<()> {
        let from = self.map.from;
        let mut defs = BTreeSet::new();
        let mut rels = BTreeSet::new();
        for t in terms
            .nodes
            .iter()
            .map(|n| &n.ty)
            .chain(terms.vars.iter().map(|v| &v.ty))
        {
            from.reached(t, &mut defs, &mut rels);
        }
        defs.extend(rels.iter().map(|&r| from.relation(r).elem_def));
        let ids = &self.map.from.ids;
        let ctor_def = |id: u64| ids.ctors.iter().find(|(_, i)| **i == id).map(|(k, _)| k.0);
        let field_def = |id: u64| ids.fields.iter().find(|(_, i)| **i == id).map(|(k, _)| k.0);
        for o in &self.ops {
            let members = o.clauses.iter().filter_map(|c| match c {
                Clause::Member { ctor } => ctor_def(*ctor),
                _ => None,
            });
            let hit = match &o.op {
                Resolved::Split { ctor, .. } | Resolved::Unnest { ctor } => {
                    ctor_def(*ctor).is_some_and(|d| defs.contains(&d))
                }
                Resolved::Merge { .. } | Resolved::Nest { .. } => {
                    members.into_iter().any(|d| defs.contains(&d))
                }
                Resolved::MoveField { field, .. } | Resolved::AlterField { field, .. } => {
                    field_def(*field).is_some_and(|d| defs.contains(&d))
                }
                Resolved::Rekey { relation, .. } => ids
                    .relations
                    .iter()
                    .any(|(r, i)| i == relation && rels.contains(r)),
                _ => false,
            };
            if hit {
                return Err(refused(
                    Some(o.text.clone()),
                    "it reaches what the operator changes, and this engine does not translate \
                     across it",
                ));
            }
        }
        Ok(())
    }

    /// Refuse a write into constructor `c` of relation `rel` that is enabled at the pin and not
    /// at the later version.
    fn writable(&self, rel: RelId, c: u32) -> R<()> {
        let to_rel = self.map.relation(rel);
        let def = self.map.from.relation(rel).elem_def;
        let (Some(to_rel), Some(k)) = (to_rel, self.map.ctor(def, c)) else {
            return Err(refused(
                self.blame_ctor(def, c),
                format!(
                    "it writes `{}`, which the current version lacks",
                    self.ctor_name(def, c)
                ),
            ));
        };
        let was = self.map.from.stages.get(&(rel, c)).copied();
        let now = self.map.to.stages.get(&(to_rel, k)).copied();
        if was.is_none() && now.is_some() {
            let id = self.map.from.ids.ctors.get(&(def, c)).copied();
            let op = self.blame(|o| match &o.op {
                Resolved::Disable { target } | Resolved::Deprecate { target } => {
                    matches!(target, crate::evolve::Target::InRelation { ctor, .. } if Some(*ctor) == id)
                }
                _ => false,
            });
            let state = now.map_or("", Stage::name);
            return Err(refused(
                op,
                format!(
                    "it writes `{}`, which is {state} now",
                    self.ctor_name(def, c)
                ),
            ));
        }
        Ok(())
    }

    fn ctor_name(&self, def: TypeId, c: u32) -> String {
        let t = self.map.from.ty(def);
        let ctor = t.ctors().get(c as usize).map_or("", |k| k.name.as_str());
        format!("{}.{ctor}", t.name)
    }

    /// Returns the normal forms of the later version's schema bodies.
    fn normal(&self) -> R<Rc<NormalProgram>> {
        if let Some(n) = self.normal.borrow().as_ref() {
            return Ok(n.clone());
        }
        let n = Rc::new(
            norm::program(self.map.to, self.to_program).map_err(|e| refused(None, e.message))?,
        );
        *self.normal.borrow_mut() = Some(n.clone());
        Ok(n)
    }

    /// Returns the fill of the operator that added field `name` of constructor `c` of `def`, at
    /// the later version, as a function of the row before it, with the operator's text.
    fn fill(&self, def: TypeId, c: u32, name: &str) -> R<Option<(RowFn, String)>> {
        let Some(&id) = self
            .map
            .to
            .ids
            .fields
            .get(&map::slot(self.map.to, def, c, name))
        else {
            return Ok(None);
        };
        let Some(o) = self
            .ops
            .iter()
            .find(|o| matches!(o.op, Resolved::AddField { field, .. } if field == id))
        else {
            return Ok(None);
        };
        let logged = &self.to_source.log[o.at.0];
        let mut before = (*logged.before).clone();
        let failed = |ds: Vec<crate::span::Diag>| {
            refused(
                Some(o.text.clone()),
                ds.into_iter()
                    .map(|d| d.message)
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        };
        for op in &logged.operators[..o.at.1] {
            before
                .apply(&logged.module, op, &logged.text)
                .map_err(failed)?;
        }
        let steps = before
            .steps(&logged.module, &logged.operators[o.at.1], &logged.text)
            .map_err(failed)?;
        Ok(steps
            .into_iter()
            .flat_map(|s| s.maps)
            .find_map(|m| match m {
                RowMap::Fill { fill, .. } => Some((fill, o.text.clone())),
                _ => None,
            }))
    }

    /// Returns whether function `f` of the pin has the same code at the later version, and its
    /// id there.
    fn function(&self, f: FnId) -> Option<FnId> {
        let id = *self.map.from.ids.functions.get(&f)?;
        let to = self
            .map
            .to
            .ids
            .functions
            .iter()
            .find(|(_, i)| **i == id)
            .map(|(k, _)| *k)?;
        self.same_code(id).then_some(to)
    }

    fn aggregate(&self, a: u32) -> Option<u32> {
        let id = *self.map.from.ids.aggregates.get(&(a as usize))?;
        let to = self
            .map
            .to
            .ids
            .aggregates
            .iter()
            .find(|(_, i)| **i == id)
            .map(|(k, _)| *k as u32)?;
        self.same_code(id).then_some(to)
    }

    fn same_code(&self, id: u64) -> bool {
        let code = |s: &Source| -> Option<Code> {
            s.functions()
                .ok()?
                .into_iter()
                .find(|f| f.id == id)
                .map(|f| f.code)
        };
        code(self.from_source).is_some_and(|c| Some(c) == code(self.to_source))
    }
}

/// Returns the first line of operator `op` of migration `text`.
fn operator_text(text: &str, op: &crate::ast::Operator) -> String {
    text.get(op.span.start as usize..op.span.end as usize)
        .and_then(|s| s.lines().next())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// The fields of a constructor value a fill reads its row through.
#[derive(Debug)]
struct Given {
    ctor: u32,
    fields: BTreeMap<String, Tm>,
}

/// Returns `kind`'s terms that stay the value of a write's row: a branch, a body, an element.
fn carries(kind: &Kind) -> Vec<bool> {
    let n = kind.children().len();
    let mut out = vec![false; n];
    match kind {
        Kind::If(..) => {
            out[1] = true;
            out[2] = true;
        }
        Kind::Let(..) | Kind::LetBang(..) | Kind::For(..) | Kind::Block(..) => out[1] = true,
        Kind::Case { .. } => out.iter_mut().skip(1).for_each(|b| *b = true),
        Kind::Singleton(_) | Kind::Union(..) => out.iter_mut().for_each(|b| *b = true),
        _ => {}
    }
    out
}
