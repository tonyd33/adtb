use super::{PResult, Parser};
use crate::ast::*;
use crate::lex::{Kw, Tok};

impl Parser<'_> {
    pub(crate) fn at_write(&self) -> bool {
        matches!(
            self.tok().tok,
            Tok::Kw(
                Kw::Insert | Kw::Update | Kw::Delete | Kw::Fail | Kw::Skip | Kw::If | Kw::Match
            ) | Tok::LParen
        )
    }

    pub(crate) fn write_expr(&mut self) -> PResult<Write> {
        let start = self.tok().span;
        let kind = match self.tok().tok {
            Tok::Kw(Kw::Insert) => self.insert()?,
            Tok::Kw(Kw::Update) => {
                self.bump();
                let binder = self.lower("a binder name")?;
                let source = if self.eat_kw(Kw::In) {
                    Some(self.or_e()?)
                } else {
                    None
                };
                let filter = if self.eat_kw(Kw::Where) {
                    Some(self.expr()?)
                } else {
                    None
                };
                self.expect_kw(Kw::Set)?;
                let value = self.expr()?;
                WriteKind::Update {
                    binder,
                    source,
                    filter,
                    value,
                }
            }
            Tok::Kw(Kw::Delete) => {
                self.bump();
                let binder = self.lower("a binder name")?;
                let source = if self.eat_kw(Kw::In) {
                    Some(self.or_e()?)
                } else {
                    None
                };
                let filter = if self.eat_kw(Kw::Where) {
                    Some(self.expr()?)
                } else {
                    None
                };
                WriteKind::Delete {
                    binder,
                    source,
                    filter,
                }
            }
            Tok::Kw(Kw::Fail) => {
                self.bump();
                WriteKind::Fail(self.or_e()?)
            }
            Tok::Kw(Kw::Skip) => {
                self.bump();
                WriteKind::Skip
            }
            Tok::LParen => {
                self.bump();
                let mut writes = vec![self.write_expr()?];
                while self.eat(Tok::Comma) {
                    writes.push(self.write_expr()?);
                }
                self.expect(Tok::RParen, "`)`")?;
                WriteKind::Seq(writes)
            }
            Tok::Kw(Kw::If) => {
                self.bump();
                let c = self.expr()?;
                self.expect_kw(Kw::Then)?;
                let a = self.write_expr()?;
                self.expect_kw(Kw::Else)?;
                let b = self.write_expr()?;
                WriteKind::If(c, Box::new(a), Box::new(b))
            }
            Tok::Kw(Kw::Match) => {
                self.bump();
                let scrutinee = self.or_e()?;
                if !self.at(Tok::Bar) {
                    return Err(self.error_here("expected `|` and a match arm"));
                }
                let mut arms = Vec::new();
                while self.eat(Tok::Bar) {
                    let (patterns, guard) = self.arm_head()?;
                    let body = self.write_expr()?;
                    arms.push(WriteArm {
                        patterns,
                        guard,
                        body,
                    });
                }
                WriteKind::Match(scrutinee, arms)
            }
            _ => return Err(self.error_here("expected a write")),
        };
        Ok(Write {
            kind,
            span: self.span_from(start),
        })
    }

    fn insert(&mut self) -> PResult<WriteKind> {
        self.expect_kw(Kw::Insert)?;
        let target = self.relation_ref()?;
        let from = self.eat_kw(Kw::From);
        let source = self.or_e()?;
        let upsert = if self.at_word("unless") {
            self.bump();
            self.expect_word("conflict")?;
            self.expect_word("on")?;
            let on = if self.eat_kw(Kw::Key) {
                ConflictTarget::Key
            } else {
                let fields = self.index_fields()?;
                let scope = if self.eat_kw(Kw::In) {
                    Some(self.ctor_scope()?)
                } else {
                    None
                };
                ConflictTarget::Fields(fields, scope)
            };
            self.expect_kw(Kw::Else)?;
            let otherwise = Box::new(self.write_expr()?);
            Some(Upsert { on, otherwise })
        } else {
            None
        };
        let rejects = if self.at_word("rejects") && self.peek(1).tok == Tok::Kw(Kw::Into) {
            self.bump();
            self.bump();
            Some(self.qlower("a relation name")?)
        } else {
            None
        };
        Ok(WriteKind::Insert {
            target,
            from,
            source,
            upsert,
            rejects,
        })
    }

    pub(crate) fn transact_block(&mut self) -> PResult<TransactBlock> {
        let start = self.tok().span;
        self.expect_kw(Kw::Transact)?;
        let mut options = Vec::new();
        loop {
            if self.at_word("retry") && self.peek(1).tok == Tok::Int {
                self.bump();
                options.push(TxnOption::Retry(self.int_lit()?));
            } else if self.eat_word("read_only") {
                let deferrable = self.eat_word("deferrable");
                options.push(TxnOption::ReadOnly { deferrable });
            } else if self.at_word("durability") {
                self.bump();
                let quorum = if self.eat_word("quorum") {
                    true
                } else {
                    self.expect_word("local")?;
                    false
                };
                options.push(TxnOption::Durability { quorum });
            } else {
                break;
            }
        }
        let mut items = Vec::new();
        loop {
            if self.eat_kw(Kw::Let) {
                let name = self.lower("a name")?;
                self.expect(Tok::Eq, "`=`")?;
                let value = self.let_value()?;
                items.push(BlockItem::Let(name, value));
            } else if self.at_write() {
                items.push(BlockItem::Write(self.write_expr()?));
            } else {
                break;
            }
        }
        let ret = if self.eat_kw(Kw::Return) {
            Some(self.expr()?)
        } else {
            None
        };
        Ok(TransactBlock {
            options,
            items,
            ret,
            span: self.span_from(start),
        })
    }

    fn let_value(&mut self) -> PResult<LetValue> {
        match self.tok().tok {
            Tok::Kw(Kw::Insert | Kw::Update | Kw::Delete | Kw::Fail | Kw::Skip) => {
                Ok(LetValue::Write(self.write_expr()?))
            }
            Tok::Kw(Kw::If | Kw::Match) | Tok::LParen => {
                if let Some(w) = self.attempt(Self::write_expr) {
                    return Ok(LetValue::Write(w));
                }
                Ok(LetValue::Expr(self.expr()?))
            }
            _ => Ok(LetValue::Expr(self.expr()?)),
        }
    }
}
