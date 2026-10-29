use super::write::{LetKind, Nested, ungroup};
use super::*;
use crate::ast::{BlockItem, LetValue, TransactBlock, TxnOption};
use crate::kernel::{Durability, TxnOptions};

/// An item of a block checked up to the items after it.
enum Pending {
    Write(Tm),
    LetWrite {
        v: Var,
        kind: LetKind,
        ty: Ty,
        nested: Vec<Nested>,
    },
    LetExpr {
        v: Var,
        value: Tm,
        nested: Vec<Nested>,
    },
}

impl Checker<'_> {
    /// Checks a `transact` block whose result, when given, is `result`.
    pub(super) fn transact(&mut self, b: &TransactBlock, result: Option<&Ty>) -> R<Out> {
        let mut opts = TxnOptions::default();
        let mut durability = false;
        for o in &b.options {
            let twice = match o {
                TxnOption::Retry(n) => opts.retry.replace(*n).is_some(),
                TxnOption::ReadOnly { deferrable } => {
                    let twice = opts.read_only;
                    opts.read_only = true;
                    opts.deferrable = *deferrable;
                    twice
                }
                TxnOption::Durability { quorum } => {
                    let level = if *quorum {
                        Durability::Quorum
                    } else {
                        Durability::Local
                    };
                    durability = true;
                    opts.durability.replace(level).is_some()
                }
            };
            if twice {
                return Err(diag(b.span, "a `transact` option is given twice"));
            }
        }
        if opts.read_only && durability {
            return Err(diag(
                b.span,
                "a `read_only` block commits no writes, so it takes no `durability`",
            ));
        }
        let read_only = std::mem::replace(&mut self.read_only, opts.read_only);
        self.scope.push();
        let r = self.items(b, result);
        self.scope.pop();
        self.read_only = read_only;
        let body = r?;
        let tm = self.add(Kind::Transact(body.tm, opts), body.ty.clone());
        Ok(Out {
            tm,
            ty: body.ty,
            rows: body.rows,
        })
    }

    fn no_writes(&self, span: Span) -> R<()> {
        if self.read_only {
            return Err(diag(span, "a `read_only` block may not write"));
        }
        Ok(())
    }

    /// Checks the items of a block, then its `return`.
    fn items(&mut self, b: &TransactBlock, result: Option<&Ty>) -> R<Out> {
        let mut pending = Vec::with_capacity(b.items.len());
        let r = self
            .item_heads(b, &mut pending)
            .and_then(|()| self.block_return(b, result));
        for p in &pending {
            if !matches!(p, Pending::Write(_)) {
                self.scope.pop();
            }
        }
        let mut k = r?;
        for p in pending.into_iter().rev() {
            k = self.item_tail(p, k);
        }
        Ok(k)
    }

    /// Checks each item of `b` up to the items after it, pushing what remains of it to `pending`.
    ///
    /// Postconditions:
    /// - A scope is pushed for each `let` in `pending`, error or not.
    fn item_heads(&mut self, b: &TransactBlock, pending: &mut Vec<Pending>) -> R<()> {
        for item in &b.items {
            match item {
                BlockItem::Write(w) => {
                    self.no_writes(w.span)?;
                    let tm = self.write(w)?;
                    pending.push(Pending::Write(tm));
                }
                BlockItem::Let(name, LetValue::Write(w)) => {
                    self.no_writes(w.span)?;
                    let w = ungroup(w);
                    self.hoist.push(Vec::new());
                    let parts = self.let_parts(w);
                    let nested = self.hoist.pop().expect("pushed");
                    let (kind, ty, rel) = parts?;
                    self.scope.push();
                    let v = self.bind(&name.text, ty.clone(), Some(rel));
                    self.proposed.insert(v);
                    pending.push(Pending::LetWrite {
                        v,
                        kind,
                        ty,
                        nested,
                    });
                }
                BlockItem::Let(name, LetValue::Expr(e)) => {
                    self.hoist.push(Vec::new());
                    let r = self.write_expr(e, None, true);
                    let nested = self.hoist.pop().expect("pushed");
                    let o = r?;
                    if !nested.is_empty() {
                        self.no_writes(e.span)?;
                    }
                    self.scope.push();
                    let v = self.bind_origin(&name.text, o.ty.clone(), self.row_origin(&o));
                    if self.mentions_proposed(o.tm) {
                        self.proposed.insert(v);
                    }
                    if self.snapshot_value(&o) {
                        self.snapshot.insert(v);
                    }
                    pending.push(Pending::LetExpr {
                        v,
                        value: o.tm,
                        nested,
                    });
                }
            }
        }
        Ok(())
    }

    /// Checks the `return` of `b`, or its absence.
    fn block_return(&mut self, b: &TransactBlock, result: Option<&Ty>) -> R<Out> {
        let Some(e) = &b.ret else {
            if let Some(t) = result
                && *t != Ty::Unit
            {
                return Err(diag(
                    b.span,
                    format!(
                        "the block returns nothing, and its result is {}",
                        self.show(t)
                    ),
                ));
            }
            return Ok(self.out(Kind::Record(Vec::new()), Ty::Unit));
        };
        self.hoist.push(Vec::new());
        let r = self.write_expr(e, result, true);
        let nested = self.hoist.pop().expect("pushed");
        let o = r?;
        if !nested.is_empty() {
            self.no_writes(e.span)?;
        }
        let ty = result.cloned().unwrap_or_else(|| o.ty.clone());
        let tm = self.block_nested(nested, o.tm, &ty);
        Ok(Out {
            tm,
            ty,
            rows: o.rows,
        })
    }

    /// Returns item `p` followed by the items after it, `k`.
    fn item_tail(&mut self, p: Pending, k: Out) -> Out {
        let tm = match p {
            Pending::Write(tm) => self.add(Kind::Block(tm, k.tm), k.ty.clone()),
            Pending::LetWrite {
                v,
                kind,
                ty,
                nested,
            } => {
                let body = match kind {
                    LetKind::Insert { rel, value, single } => {
                        let vt = self.var_tm(v);
                        let rows = if single { self.singleton(vt, &ty) } else { vt };
                        let ins = self.change(Kind::Insert(rel, rows));
                        let blk = self.add(Kind::Block(ins, k.tm), k.ty.clone());
                        self.add(Kind::Let(v, value, blk), k.ty.clone())
                    }
                    LetKind::Delete { rel, rows } => {
                        let vt = self.var_tm(v);
                        let del = self.change(Kind::Delete(rel, vt));
                        let blk = self.add(Kind::Block(del, k.tm), k.ty.clone());
                        self.add(Kind::Let(v, rows, blk), k.ty.clone())
                    }
                    LetKind::Bang(write) => self.add(Kind::LetBang(v, write, k.tm), k.ty.clone()),
                };
                self.block_nested(nested, body, &k.ty)
            }
            Pending::LetExpr { v, value, nested } => {
                let tm = self.add(Kind::Let(v, value, k.tm), k.ty.clone());
                self.block_nested(nested, tm, &k.ty)
            }
        };
        Out { tm, ..k }
    }
}

/// A checked registered transaction: its term, a `Lam` over its parameters of a `Transact`.
#[derive(Debug)]
pub struct Txn {
    pub terms: Terms,
    pub term: Tm,
}

/// Type checks a registered transaction of a bundle that reads `module`, against `program`, the checked `schema`.
pub fn check_txn(
    schema: &Schema,
    program: &Program,
    module: ModuleId,
    t: &ast::TxnDecl,
) -> Result<Txn, Diag> {
    check_txn_in(schema, program, Visible::module(module), t)
}

/// Type checks a registered transaction of a bundle that reads the modules of `vis`, as [`check_txn`] does.
pub fn check_txn_in(
    schema: &Schema,
    program: &Program,
    vis: Visible,
    t: &ast::TxnDecl,
) -> Result<Txn, Diag> {
    let mut c = Checker::new(schema, vis, Context::Function);
    let term = c.txn(t)?;
    let txn = Txn {
        terms: c.terms,
        term,
    };
    crate::analysis::txn_diag(schema, program, &txn, t.span)?;
    Ok(txn)
}

impl Checker<'_> {
    fn txn(&mut self, t: &ast::TxnDecl) -> R<Tm> {
        if let Some(l) = &t.limit {
            self.check(l, &Ty::Base(Base::Duration))?;
        }
        let result = self.resolve_type(&t.result)?;
        if result.mentions_fn() {
            return Err(diag(
                t.result.span,
                "a registered transaction never returns a function",
            ));
        }
        self.scope.push();
        let mut vars = Vec::new();
        for p in &t.params {
            let pty = self.resolve_type(&p.ty)?;
            if pty.mentions_fn() {
                return Err(diag(
                    p.ty.span,
                    "a parameter of a registered transaction never has a function type",
                ));
            }
            vars.push(self.bind(&p.name.text, pty, None));
        }
        let body = self.transact(&t.body, Some(&result));
        self.scope.pop();
        let body = body?;
        Ok(self.lams(vars, body.tm, result))
    }
}
