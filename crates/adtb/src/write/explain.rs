//! The operators of a statement for `explain`: one per write form, over the plan of each query
//! the statement evaluates.

use adtb_lang::kernel::{Kind, Tm};
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
    /// Returns the operators of the statement.
    pub fn explain(&self) -> Result<Node, Error> {
        let d = Describe::new(self.schema(), &self.db.catalog);
        let root = self.root();
        match self.terms().kind(root) {
            Kind::Transact(b, _) => {
                let b = *b;
                Ok(Node::new("Transact", "", vec![self.block_node(&d, b)?]))
            }
            _ => self.write_node(&d, root),
        }
    }

    fn is_write(&self, t: Tm) -> bool {
        *self.terms().ty(t) == Ty::Change
    }

    fn rel_name(&self, r: RelId) -> String {
        self.schema().relation(r).name.clone()
    }

    fn query_node(&self, d: &Describe<'_>, t: Tm) -> Result<Node, Error> {
        Ok(d.plan(&self.plan_of(t)?))
    }

    fn block_node(&self, d: &Describe<'_>, t: Tm) -> Result<Node, Error> {
        match self.terms().kind(t).clone() {
            Kind::Block(w, mut k) => {
                let mut steps = vec![self.write_node(d, w)?];
                while let Kind::Block(w, next) = self.terms().kind(k).clone() {
                    steps.push(self.write_node(d, w)?);
                    k = next;
                }
                steps.push(self.block_node(d, k)?);
                Ok(Node::new("Then", "", steps))
            }
            Kind::LetBang(_, w, k) => {
                let bound = self.write_node(d, w)?;
                let rest = self.block_node(d, k);
                Ok(Node::new("Let", "", vec![bound, rest?]))
            }
            Kind::Let(_, e, k) => {
                let bound = self.query_node(d, e)?;
                let rest = self.block_node(d, k);
                Ok(Node::new("Let", "", vec![bound, rest?]))
            }
            _ => self.write_node(d, t),
        }
    }

    /// Returns the operators of `t`, a write or, outside a write, a query.
    fn write_node(&self, d: &Describe<'_>, t: Tm) -> Result<Node, Error> {
        let kind = self.terms().kind(t).clone();
        let leaf = |kind: &str, r: RelId, rows: Tm| -> Result<Node, Error> {
            Ok(Node::new(
                kind,
                self.rel_name(r),
                vec![self.query_node(d, rows)?],
            ))
        };
        match kind {
            Kind::Skip => Ok(Node::new("Skip", "", Vec::new())),
            Kind::Insert(r, rows) => leaf("Insert", r, rows),
            Kind::Delete(r, rows) => leaf("Delete", r, rows),
            Kind::Replace(r, rows, _) => leaf("Replace", r, rows),
            Kind::Upsert { rel, rows, .. } => leaf("Upsert", rel, rows),
            Kind::Rejects { into, insert } => Ok(Node::new(
                "Rejects into",
                self.rel_name(into),
                vec![self.write_node(d, insert)?],
            )),
            Kind::Par(ws) => {
                let mut children = Vec::with_capacity(ws.len());
                for w in ws {
                    children.push(self.write_node(d, w)?);
                }
                Ok(Node::new("Together", "", children))
            }
            Kind::Fail(_) => Ok(Node::new("Fail", "", Vec::new())),
            Kind::If(_, a, b) if self.is_write(t) => {
                let a = self.write_node(d, a)?;
                let b = self.write_node(d, b)?;
                Ok(Node::new("If", "", vec![a, b]))
            }
            Kind::Case { scrutinee, arms } if self.is_write(t) => {
                let ty = self.terms().ty(scrutinee).clone();
                let mut n = Node::new("Route by constructor", "", Vec::new());
                for (c, _, w) in arms {
                    let child = self.write_node(d, w);
                    n.arms.push(Arm {
                        ctor: c,
                        name: ctor_name(self.schema(), &ty, c),
                        child: child?,
                    });
                }
                Ok(n)
            }
            Kind::Let(_, e, w) if self.is_write(t) => {
                let bound = self.query_node(d, e)?;
                let rest = self.write_node(d, w);
                Ok(Node::new("Let", "", vec![bound, rest?]))
            }
            _ => self.query_node(d, t),
        }
    }
}
