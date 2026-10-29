//! The operators of a statement for `explain`: one per write form, over the plan of each query
//! the statement evaluates.

use adtb_lang::kernel::{Kind, Tm, Var};
use adtb_lang::types::{RelId, Ty};

use crate::plan::explain::{Arm, Describe, Node, ctor_name};
use crate::runtime::Runtime;

use super::delta::CheckPlan;
use super::eval::Stmt;
use super::{Database, Error};

impl<R: Runtime> Database<R> {
    /// Returns each incremental check the constraint or check `name`, by qualified name, is
    /// admitted with, or `None` when the schema has no constraint or check `name`.
    pub fn checks(&self, name: &str) -> Option<Vec<CheckPlan>> {
        let d = self.denials.iter().find(|d| d.name == name)?;
        Some(
            d.deltas
                .iter()
                .map(|(delta, _)| delta.explain(&self.schema, &self.catalog))
                .collect(),
        )
    }
}

impl<R: Runtime> Stmt<'_, R> {
    /// Returns the operators of the statement, its parameters `params`.
    pub fn explain(&self, params: &[Var]) -> Result<Node, Error> {
        let mut scope: Vec<Var> = params.to_vec();
        scope.extend(self.input_vars());
        let d = Describe::new(self.schema(), &self.db.catalog);
        let root = self.root();
        match self.terms().kind(root) {
            Kind::Transact(b, _) => {
                let b = *b;
                Ok(Node::new(
                    "Transact",
                    "",
                    vec![self.block_node(&d, b, &mut scope)?],
                ))
            }
            _ => self.write_node(&d, root, &mut scope),
        }
    }

    fn is_write(&self, t: Tm) -> bool {
        *self.terms().ty(t) == Ty::Change
    }

    fn rel_name(&self, r: RelId) -> String {
        self.schema().relation(r).name.clone()
    }

    fn query_node(&self, d: &Describe<'_>, t: Tm, scope: &[Var]) -> Result<Node, Error> {
        Ok(d.plan(&self.plan_of(t, scope)?))
    }

    fn block_node(&self, d: &Describe<'_>, t: Tm, scope: &mut Vec<Var>) -> Result<Node, Error> {
        match self.terms().kind(t).clone() {
            Kind::Block(w, k) => {
                let first = self.write_node(d, w, scope)?;
                let rest = self.block_node(d, k, scope)?;
                Ok(Node::new("Then", "", vec![first, rest]))
            }
            Kind::LetBang(x, w, k) => {
                let bound = self.write_node(d, w, scope)?;
                scope.push(x);
                let rest = self.block_node(d, k, scope);
                scope.pop();
                Ok(Node::new("Let", "", vec![bound, rest?]))
            }
            Kind::Let(x, e, k) => {
                let bound = self.query_node(d, e, scope)?;
                scope.push(x);
                let rest = self.block_node(d, k, scope);
                scope.pop();
                Ok(Node::new("Let", "", vec![bound, rest?]))
            }
            _ => self.write_node(d, t, scope),
        }
    }

    /// Returns the operators of `t`, a write or, outside a write, a query.
    fn write_node(&self, d: &Describe<'_>, t: Tm, scope: &mut Vec<Var>) -> Result<Node, Error> {
        let kind = self.terms().kind(t).clone();
        let leaf = |kind: &str, r: RelId, rows: Tm, scope: &[Var]| -> Result<Node, Error> {
            Ok(Node::new(
                kind,
                self.rel_name(r),
                vec![self.query_node(d, rows, scope)?],
            ))
        };
        match kind {
            Kind::Skip => Ok(Node::new("Skip", "", Vec::new())),
            Kind::Insert(r, rows) => leaf("Insert", r, rows, scope),
            Kind::Delete(r, rows) => leaf("Delete", r, rows, scope),
            Kind::Replace(r, rows, _) => leaf("Replace", r, rows, scope),
            Kind::Upsert { rel, rows, .. } => leaf("Upsert", rel, rows, scope),
            Kind::Rejects { into, insert } => Ok(Node::new(
                "Rejects into",
                self.rel_name(into),
                vec![self.write_node(d, insert, scope)?],
            )),
            Kind::Par(ws) => {
                let mut children = Vec::with_capacity(ws.len());
                for w in ws {
                    children.push(self.write_node(d, w, scope)?);
                }
                Ok(Node::new("Together", "", children))
            }
            Kind::Fail(_) => Ok(Node::new("Fail", "", Vec::new())),
            Kind::If(_, a, b) if self.is_write(t) => {
                let a = self.write_node(d, a, scope)?;
                let b = self.write_node(d, b, scope)?;
                Ok(Node::new("If", "", vec![a, b]))
            }
            Kind::Case { scrutinee, arms } if self.is_write(t) => {
                let ty = self.terms().ty(scrutinee).clone();
                let mut n = Node::new("Route by constructor", "", Vec::new());
                for (c, x, w) in arms {
                    scope.push(x);
                    let child = self.write_node(d, w, scope);
                    scope.pop();
                    n.arms.push(Arm {
                        ctor: c,
                        name: ctor_name(self.schema(), &ty, c),
                        child: child?,
                    });
                }
                Ok(n)
            }
            Kind::Let(x, e, w) if self.is_write(t) => {
                let bound = self.query_node(d, e, scope)?;
                scope.push(x);
                let rest = self.write_node(d, w, scope);
                scope.pop();
                Ok(Node::new("Let", "", vec![bound, rest?]))
            }
            _ => self.query_node(d, t, scope),
        }
    }
}
