use super::{Attempt, PResult, Parser};
use crate::ast::*;
use crate::lex::{self, Kw, Tok};
use crate::span::Span;

impl Parser<'_> {
    fn mk(&self, kind: ExprKind, start: Span) -> Expr {
        Expr {
            kind,
            span: self.span_from(start),
        }
    }

    fn binary(&self, op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
        let span = lhs.span.to(rhs.span);
        Expr {
            kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)),
            span,
        }
    }

    pub(crate) fn expr(&mut self) -> PResult<Expr> {
        self.operators(Level::Union)
    }

    pub(crate) fn or_e(&mut self) -> PResult<Expr> {
        self.operators(Level::Or)
    }

    pub(crate) fn add_e(&mut self) -> PResult<Expr> {
        self.operators(Level::Add)
    }

    fn infix(&self) -> Option<(BinOp, Level)> {
        Some(match self.tok().tok {
            Tok::Kw(Kw::Union) => (BinOp::Union, Level::Union),
            Tok::Kw(Kw::Or) => (BinOp::Or, Level::Or),
            Tok::Kw(Kw::And) => (BinOp::And, Level::And),
            Tok::EqEq => (BinOp::Eq, Level::Cmp),
            Tok::NotEq => (BinOp::Ne, Level::Cmp),
            Tok::Lt => (BinOp::Lt, Level::Cmp),
            Tok::Le => (BinOp::Le, Level::Cmp),
            Tok::Gt => (BinOp::Gt, Level::Cmp),
            Tok::Ge => (BinOp::Ge, Level::Cmp),
            Tok::Plus => (BinOp::Add, Level::Add),
            Tok::Minus => (BinOp::Sub, Level::Add),
            Tok::PlusPlus => (BinOp::Concat, Level::Add),
            Tok::Star => (BinOp::Mul, Level::Mul),
            Tok::Slash => (BinOp::Div, Level::Mul),
            Tok::Percent => (BinOp::Rem, Level::Mul),
            _ => return None,
        })
    }

    /// Parses an operand and every infix operator of level `min` or tighter that follows it.
    fn operators(&mut self, min: Level) -> PResult<Expr> {
        if min <= Level::And {
            return self.associative(min);
        }
        self.chain(
            |p| {
                if min <= Level::Not && p.at_kw(Kw::Not) {
                    p.not_e()
                } else {
                    p.prefix_e()
                }
            },
            |p| p.infix().is_some_and(|(_, level)| level >= min),
            |p, lhs| {
                let (op, level) = p.infix().expect("an infix operator follows");
                p.bump();
                let rhs = p.operators(level.operand())?;
                if level == Level::Cmp && matches!(p.infix(), Some((_, Level::Cmp))) {
                    return Err(p.error_here("comparisons do not chain; parenthesise one"));
                }
                Ok(p.binary(op, lhs, rhs))
            },
        )
    }

    /// Parses a chain of the operator of `level`, which is `union`, `or` or `and`, as a
    /// balanced tree of its operands in order.
    ///
    /// Postconditions:
    /// - The chain's height is the height of that tree over its operands' heights, and the
    ///   nesting depth is unchanged.
    fn associative(&mut self, level: Level) -> PResult<Expr> {
        let base = self.depth;
        let outer = self.peak;
        let mut op = None;
        let mut parts = Vec::new();
        loop {
            self.peak = base;
            let e = self.operators(level.operand())?;
            parts.push((e, self.peak - base));
            match self.infix() {
                Some((o, l)) if l == level => {
                    op = Some(o);
                    self.bump();
                }
                _ => break,
            }
        }
        let (e, height) = match op {
            Some(op) => self.balanced(op, parts),
            None => parts.pop().expect("a chain has an operand"),
        };
        self.reach(base + height)?;
        self.peak = outer.max(base + height);
        Ok(e)
    }

    /// Returns `parts` joined by `op` as a balanced tree, nesting to the left where it is
    /// uneven, with its height.
    fn balanced(&self, op: BinOp, mut parts: Vec<(Expr, u32)>) -> (Expr, u32) {
        if parts.len() == 1 {
            return parts.pop().expect("a balanced tree has a part");
        }
        let right = parts.split_off(parts.len().div_ceil(2));
        let (l, lh) = self.balanced(op, parts);
        let (r, rh) = self.balanced(op, right);
        (self.binary(op, l, r), lh.max(rh) + 1)
    }

    fn not_e(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        self.expect_kw(Kw::Not)?;
        let e = self.nested(|p| p.operators(Level::Not))?;
        Ok(self.mk(ExprKind::Unary(UnOp::Not, Box::new(e)), start))
    }

    pub(crate) fn prefix_e(&mut self) -> PResult<Expr> {
        self.nested(|p| {
            let start = p.tok().span;
            let op = match p.tok().tok {
                Tok::Minus => UnOp::Neg,
                Tok::Kw(Kw::Key) => UnOp::Key,
                Tok::Kw(Kw::Try) => UnOp::Try,
                _ => return p.postfix_e(),
            };
            p.bump();
            let e = p.prefix_e()?;
            Ok(p.mk(ExprKind::Unary(op, Box::new(e)), start))
        })
    }

    fn at_adjacent_paren(&self) -> bool {
        self.at(Tok::LParen) && !self.tok().spaced
    }

    fn postfix_e(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        self.chain(Self::primary, Self::at_postfix, |p, e| p.postfix(e, start))
    }

    fn at_postfix(&mut self) -> bool {
        self.at_ctor_sel_after_dot()
            || self.at(Tok::DotLt)
            || self.at(Tok::Dot) && matches!(self.peek(1).tok, Tok::Lower | Tok::Kw(_))
    }

    fn postfix(&mut self, e: Expr, start: Span) -> PResult<Expr> {
        if self.at_ctor_sel_after_dot() {
            self.select_postfix(e, start)
        } else if self.at(Tok::Dot) && self.peek(1).tok == Tok::Kw(Kw::Key) && {
            let p = self.peek(2);
            p.tok == Tok::LParen && !p.spaced
        } {
            self.key_postfix(e, start)
        } else if self.at(Tok::Dot) {
            self.field_postfix(e, start)
        } else {
            self.backlink_postfix(e, start)
        }
    }

    fn select_postfix(&mut self, e: Expr, start: Span) -> PResult<Expr> {
        self.bump();
        let sel = self.ctor_sel()?;
        Ok(self.mk(ExprKind::Select(Box::new(e), sel), start))
    }

    fn key_postfix(&mut self, e: Expr, start: Span) -> PResult<Expr> {
        self.bump();
        self.bump();
        let args = self.key_args()?;
        Ok(self.mk(ExprKind::KeyCall(Box::new(e), args), start))
    }

    fn field_postfix(&mut self, e: Expr, start: Span) -> PResult<Expr> {
        self.bump();
        let field = self.field_word()?;
        if self.at_adjacent_paren() {
            return Err(self
                .error_here("`x.f(...)` is a call only as `T.f(...)` on a type or `r.key(...)`"));
        }
        Ok(self.mk(ExprKind::Field(Box::new(e), field), start))
    }

    fn backlink_postfix(&mut self, e: Expr, start: Span) -> PResult<Expr> {
        self.bump();
        let rstart = self.tok().span;
        let relation = self.qlower("a relation name")?;
        let sel = if self.at_ctor_sel_after_dot() {
            self.bump();
            Some(self.ctor_sel()?)
        } else {
            None
        };
        let rel = RelationRef {
            relation,
            sel,
            span: self.span_from(rstart),
        };
        self.expect(Tok::Dot, "`.` and the field of the backlink")?;
        let field = self.field_word()?;
        Ok(self.mk(ExprKind::Backlink(Box::new(e), rel, field), start))
    }

    fn key_args(&mut self) -> PResult<KeyArgs> {
        self.expect(Tok::LParen, "`(`")?;
        let args = if self.at_field_word() && self.peek(1).tok == Tok::Colon {
            let mut named = Vec::new();
            loop {
                let name = self.field_word()?;
                self.expect(Tok::Colon, "`:`")?;
                named.push((name, self.expr()?));
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
            KeyArgs::Named(named)
        } else {
            KeyArgs::Value(Box::new(self.expr()?))
        };
        self.expect(Tok::RParen, "`)`")?;
        Ok(args)
    }

    pub(crate) fn call_args(&mut self) -> PResult<CallArgs> {
        self.expect(Tok::LParen, "`(`")?;
        if self.eat(Tok::RParen) {
            return Ok(CallArgs::List(Vec::new()));
        }
        let first = self.expr()?;
        if self.eat_kw(Kw::For) {
            let (generators, filter) = self.scoped(Self::filtered_generators)?;
            self.expect(Tok::RParen, "`)`")?;
            return Ok(CallArgs::Aggregate {
                expr: Box::new(first),
                generators,
                filter,
            });
        }
        let mut args = vec![first];
        while self.eat(Tok::Comma) {
            args.push(self.expr()?);
        }
        self.expect(Tok::RParen, "`)`")?;
        Ok(CallArgs::List(args))
    }

    fn maybe_pins(&mut self) -> PResult<Option<Version>> {
        if self.at_word("pins") && self.peek(1).tok == Tok::Int {
            self.bump();
            Ok(Some(self.version()?))
        } else {
            Ok(None)
        }
    }

    fn lit(&mut self) -> PResult<Lit> {
        let t = self.bump();
        let text = self.text_of(t).to_string();
        Ok(match t.tok {
            Tok::Int => Lit::Int(text),
            Tok::Decimal => Lit::Decimal(text),
            Tok::Float => Lit::Float(text),
            Tok::Text => Lit::Text(lex::decode_text(&text)),
            Tok::Duration => {
                let split = text
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(text.len());
                let amount = text[..split]
                    .parse()
                    .map_err(|_| self.error_at(t.span, "duration out of range"))?;
                Lit::Duration {
                    amount,
                    unit: text[split..].to_string(),
                }
            }
            Tok::Kw(Kw::True) => Lit::Bool(true),
            Tok::Kw(Kw::False) => Lit::Bool(false),
            _ => return Err(self.error_at(t.span, "expected a literal")),
        })
    }

    fn starts_expr(&self, n: usize) -> bool {
        let t = self.peek(n);
        match t.tok {
            Tok::Lower
            | Tok::Upper
            | Tok::Int
            | Tok::Decimal
            | Tok::Float
            | Tok::Text
            | Tok::Duration
            | Tok::LParen
            | Tok::LBrace
            | Tok::LBracket => true,
            Tok::Kw(kw) => matches!(
                kw,
                Kw::True
                    | Kw::False
                    | Kw::From
                    | Kw::Match
                    | Kw::Fix
                    | Kw::Fn
                    | Kw::Exists
                    | Kw::Top
                    | Kw::If
                    | Kw::Key
                    | Kw::Try
                    | Kw::Not
            ),
            _ => false,
        }
    }

    fn primary(&mut self) -> PResult<Expr> {
        let t = self.tok();
        match t.tok {
            Tok::Int
            | Tok::Decimal
            | Tok::Float
            | Tok::Text
            | Tok::Duration
            | Tok::Kw(Kw::True | Kw::False) => self.lit_expr(),
            Tok::Kw(Kw::From) => self.comprehension_expr(),
            Tok::Kw(Kw::Match) => self.match_expr(),
            Tok::Kw(Kw::Fix) => self.fix_expr(),
            Tok::Kw(Kw::Fn) => self.lambda(),
            Tok::Kw(Kw::Exists) => self.exists_expr(),
            Tok::Kw(Kw::Top) => self.top_expr(),
            Tok::Kw(Kw::If) => self.if_expr(),
            Tok::Lower
                if self.at_word("at")
                    && (self.peek_word(1, "commit") || self.peek_word(1, "instant")) =>
            {
                self.at_expr()
            }
            Tok::Lower if self.at_word("reject") && self.peek(1).spaced && self.starts_expr(1) => {
                self.reject_expr()
            }
            Tok::Lower if self.at_qupper() => self.upper_primary(),
            Tok::Lower => self.name_or_call(),
            Tok::Upper => self.upper_primary(),
            Tok::LParen => self.paren_primary(),
            Tok::LBrace => self.record_primary(),
            Tok::LBracket => self.set_literal(),
            Tok::Dot if self.at_elision() => {
                let start = t.span;
                self.bump();
                self.bump();
                self.bump();
                Ok(self.mk(ExprKind::Elided, start))
            }
            _ => Err(self.error_here("expected an expression")),
        }
    }

    fn lit_expr(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        let lit = self.lit()?;
        Ok(self.mk(ExprKind::Lit(lit), start))
    }

    fn comprehension_expr(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        let c = self.comprehension()?;
        Ok(self.mk(ExprKind::Comprehension(Box::new(c)), start))
    }

    fn match_expr(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        self.bump();
        let scrutinee = self.or_e()?;
        let arms = self.arms()?;
        Ok(self.mk(ExprKind::Match(Some(Box::new(scrutinee)), arms), start))
    }

    fn exists_expr(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        self.bump();
        let (gens, filter) = self.scoped(Self::filtered_generators)?;
        Ok(self.mk(ExprKind::Exists(gens, filter), start))
    }

    fn filtered_generators(&mut self) -> PResult<(Vec<Generator>, Option<Box<Expr>>)> {
        let gens = self.generators()?;
        let filter = if self.eat_kw(Kw::Where) {
            Some(Box::new(self.expr()?))
        } else {
            None
        };
        Ok((gens, filter))
    }

    fn top_expr(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        self.bump();
        let count = self.add_e()?;
        self.expect_kw(Kw::By)?;
        let keys = self.order_keys()?;
        self.expect(Tok::LParen, "`(` and the operand of `top`")?;
        if !self.at_kw(Kw::From) {
            return Err(self.error_here(
                "expected the operand of `top`, a comprehension starting with `from`",
            ));
        }
        let body = self.comprehension()?;
        self.expect(Tok::RParen, "`)`")?;
        Ok(self.mk(
            ExprKind::Top {
                count: Box::new(count),
                keys,
                body: Box::new(body),
            },
            start,
        ))
    }

    fn if_expr(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        self.bump();
        let c = self.expr()?;
        self.expect_kw(Kw::Then)?;
        let a = self.expr()?;
        self.expect_kw(Kw::Else)?;
        let b = self.expr()?;
        Ok(self.mk(ExprKind::If(Box::new(c), Box::new(a), Box::new(b)), start))
    }

    fn at_expr(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        self.bump();
        let instant = self.bump();
        let instant = self.is_word(instant, "instant");
        let point = self.add_e()?;
        let pins = self.maybe_pins()?;
        self.expect(Tok::LBrace, "`{`")?;
        let body = self.expr()?;
        self.expect(Tok::RBrace, "`}`")?;
        Ok(self.mk(
            ExprKind::At {
                instant,
                point: Box::new(point),
                pins,
                body: Box::new(body),
            },
            start,
        ))
    }

    fn reject_expr(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        self.bump();
        let e = self.expr()?;
        Ok(self.mk(ExprKind::Reject(Box::new(e)), start))
    }

    fn name_or_call(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        let name = self.qlower("a name")?;
        if self.at_adjacent_paren() {
            let args = self.call_args()?;
            let pins = self.maybe_pins()?;
            return Ok(self.mk(
                ExprKind::Call {
                    callee: name,
                    args,
                    pins,
                },
                start,
            ));
        }
        Ok(self.mk(ExprKind::Name(name), start))
    }

    fn record_primary(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        let r = self.record_expr()?;
        Ok(self.mk(ExprKind::Record(r), start))
    }

    fn set_literal(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        self.bump();
        let mut elems = Vec::new();
        if !self.at(Tok::RBracket) {
            loop {
                elems.push(self.expr()?);
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::RBracket, "`]`")?;
        Ok(self.mk(ExprKind::Set(elems), start))
    }

    /// A primary that starts with a type or constructor name.
    fn upper_primary(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        let path = self.qupper("a type or constructor name")?;
        if self.at_adjacent_paren() {
            let args = self.call_args()?;
            let pins = self.maybe_pins()?;
            return Ok(self.mk(
                ExprKind::Call {
                    callee: path,
                    args,
                    pins,
                },
                start,
            ));
        }
        if self.at(Tok::Dot) && self.peek(1).tok == Tok::Lower && {
            let p = self.peek(2);
            p.tok == Tok::LParen && !p.spaced
        } {
            self.bump();
            let func = self.lower("a function name")?;
            let args = self.call_args()?;
            let span = path.span();
            let ty = Type {
                kind: TypeKind::Named {
                    name: path,
                    sel: None,
                    args: Vec::new(),
                },
                span,
            };
            return Ok(self.mk(ExprKind::TypeCall { ty, func, args }, start));
        }
        let ctor = if self.at(Tok::Dot) && self.peek(1).tok == Tok::Upper {
            self.bump();
            Some(self.upper("a constructor name")?)
        } else {
            None
        };
        let record = if self.at(Tok::LBrace) {
            Some(self.record_expr()?)
        } else {
            None
        };
        Ok(self.mk(ExprKind::Ctor { path, ctor, record }, start))
    }

    fn at_write_start(&self, n: usize) -> bool {
        matches!(
            self.peek(n).tok,
            Tok::Kw(Kw::Insert | Kw::Update | Kw::Delete | Kw::Fail | Kw::Skip)
        )
    }

    fn paren_primary(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        if self.peek(1).tok == Tok::RParen {
            self.bump();
            self.bump();
            return Ok(self.mk(ExprKind::Unit, start));
        }
        if let Some(e) = self.paren_write()? {
            return Ok(e);
        }
        if let Some(e) = self.attempt(Attempt::TypeCall, Self::paren_type_call) {
            return Ok(e);
        }
        self.paren_expr()
    }

    fn paren_write(&mut self) -> PResult<Option<Expr>> {
        let start = self.tok().span;
        let w = if self.at_write_start(1) {
            self.write_expr()?
        } else if matches!(self.peek(1).tok, Tok::Kw(Kw::If | Kw::Match) | Tok::LParen)
            && let Some(w) = self.attempt(Attempt::Write, Self::write_expr)
        {
            w
        } else {
            return Ok(None);
        };
        Ok(Some(self.mk(ExprKind::Write(Box::new(w)), start)))
    }

    fn paren_expr(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        self.bump();
        let inner = self.expr()?;
        self.expect(Tok::RParen, "`)`")?;
        Ok(Expr {
            kind: inner.kind,
            span: self.span_from(start),
        })
    }

    /// `(Tag Identity).parse(...)`.
    fn paren_type_call(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        self.expect(Tok::LParen, "`(`")?;
        let ty = self.ty()?;
        self.expect(Tok::RParen, "`)`")?;
        self.expect(Tok::Dot, "`.`")?;
        let func = self.lower("a function name")?;
        if !self.at_adjacent_paren() {
            return Err(self.error_here("expected `(`"));
        }
        let args = self.call_args()?;
        Ok(self.mk(ExprKind::TypeCall { ty, func, args }, start))
    }

    pub(crate) fn record_expr(&mut self) -> PResult<RecordExpr> {
        let start = self.tok().span;
        self.expect(Tok::LBrace, "`{`")?;
        if self.opts.elisions && self.at(Tok::Dot) && self.peek(1).tok == Tok::Dot {
            self.bump();
            self.bump();
            if self.at(Tok::Dot) {
                self.bump();
            }
            self.expect(Tok::RBrace, "`}`")?;
            return Ok(RecordExpr::Elided(self.span_from(start)));
        }
        if self.eat(Tok::RBrace) {
            return Ok(RecordExpr::Fields(Vec::new(), self.span_from(start)));
        }
        let with_base = if self.at_field_word() && self.peek(1).tok == Tok::Colon {
            None
        } else {
            let base = self.expr()?;
            self.expect_kw(Kw::With)?;
            Some(base)
        };
        let mut fields = Vec::new();
        loop {
            let name = self.field_word()?;
            self.expect(Tok::Colon, "`:`")?;
            fields.push((name, self.expr()?));
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RBrace, "`}`")?;
        let span = self.span_from(start);
        Ok(match with_base {
            Some(base) => RecordExpr::With(Box::new(base), fields, span),
            None => RecordExpr::Fields(fields, span),
        })
    }

    /// Parses generators, each one level deeper until the end of the enclosing
    /// [`Parser::scoped`].
    pub(crate) fn generators(&mut self) -> PResult<Vec<Generator>> {
        let mut gens = Vec::new();
        loop {
            self.deeper()?;
            let binder = self.lower("a binder name")?;
            self.expect_kw(Kw::In)?;
            let source = self.or_e()?;
            gens.push(Generator { binder, source });
            if !(self.at(Tok::Comma)
                && self.peek(1).tok == Tok::Lower
                && self.peek(2).tok == Tok::Kw(Kw::In))
            {
                break;
            }
            self.bump();
        }
        Ok(gens)
    }

    fn clauses(&mut self) -> PResult<Vec<Clause>> {
        let mut clauses = Vec::new();
        loop {
            if self.at_kw(Kw::Where) {
                self.deeper()?;
            }
            if self.eat_kw(Kw::From) {
                clauses.push(Clause::From(self.generators()?));
            } else if self.eat_kw(Kw::Where) {
                clauses.push(Clause::Where(self.expr()?));
            } else {
                return Ok(clauses);
            }
        }
    }

    fn comprehension(&mut self) -> PResult<Comprehension> {
        self.scoped(Self::comprehension_in_scope)
    }

    fn comprehension_in_scope(&mut self) -> PResult<Comprehension> {
        let clauses = self.clauses()?;
        let group = if self.eat_kw(Kw::Group) {
            let value = self.or_e()?;
            self.expect_kw(Kw::By)?;
            let by = self.or_e()?;
            self.expect_kw(Kw::Into)?;
            let into = self.lower("a group name")?;
            let after = self.clauses()?;
            Some((Group { value, by, into }, after))
        } else {
            None
        };
        self.expect_kw(Kw::Select)?;
        let select = self.expr()?;
        let order = if self.eat_kw(Kw::Order) {
            self.expect_kw(Kw::By)?;
            let keys = self.order_keys()?;
            let mut take = None;
            let mut after = None;
            if self.eat_word("take") {
                take = Some(self.add_e()?);
                if self.eat_word("after") {
                    after = Some(self.add_e()?);
                }
            }
            Some(OrderClause { keys, take, after })
        } else {
            None
        };
        Ok(Comprehension {
            clauses,
            group,
            select,
            order,
        })
    }

    pub(crate) fn order_keys(&mut self) -> PResult<Vec<OrderKey>> {
        if self.eat(Tok::LParen) {
            let mut keys = vec![self.order_key()?];
            while self.eat(Tok::Comma) {
                keys.push(self.order_key()?);
            }
            self.expect(Tok::RParen, "`)`")?;
            Ok(keys)
        } else {
            Ok(vec![self.order_key()?])
        }
    }

    fn order_key(&mut self) -> PResult<OrderKey> {
        let expr = self.add_e()?;
        let desc = if self.eat_word("desc") {
            true
        } else {
            self.eat_word("asc");
            false
        };
        Ok(OrderKey { expr, desc })
    }

    pub(crate) fn arms(&mut self) -> PResult<Vec<Arm>> {
        if !self.at(Tok::Bar) {
            return Err(self.error_here("expected `|` and a match arm"));
        }
        let mut arms = Vec::new();
        while self.eat(Tok::Bar) {
            let (patterns, guard) = self.arm_head()?;
            let body = self.expr()?;
            arms.push(Arm {
                patterns,
                guard,
                body,
            });
        }
        Ok(arms)
    }

    pub(crate) fn arm_head(&mut self) -> PResult<(Vec<Pattern>, Option<Expr>)> {
        let mut patterns = vec![self.pattern()?];
        while self.eat(Tok::Bar) {
            patterns.push(self.pattern()?);
        }
        let guard = if self.eat_kw(Kw::If) {
            Some(self.expr()?)
        } else {
            None
        };
        self.expect(Tok::Arrow, "`->`")?;
        Ok((patterns, guard))
    }

    pub(crate) fn pattern(&mut self) -> PResult<Pattern> {
        self.nested(Self::pattern_at)
    }

    fn pattern_at(&mut self) -> PResult<Pattern> {
        let start = self.tok().span;
        let kind = match self.tok().tok {
            Tok::Underscore => {
                self.bump();
                PatternKind::Wildcard
            }
            Tok::Lower if !self.at_qupper() => {
                let t = self.bump();
                PatternKind::Bind(self.ident(t))
            }
            Tok::Lower | Tok::Upper => {
                let path = self.qupper("a constructor name")?;
                let ctor = if self.at(Tok::Dot) && self.peek(1).tok == Tok::Upper {
                    self.bump();
                    Some(self.upper("a constructor name")?)
                } else {
                    None
                };
                let payload = match self.tok().tok {
                    Tok::Underscore => {
                        self.bump();
                        Some(Payload::Wildcard)
                    }
                    Tok::Lower => {
                        let t = self.bump();
                        Some(Payload::Bind(self.ident(t)))
                    }
                    Tok::LBrace => Some(Payload::Record(self.record_pat()?)),
                    _ => None,
                };
                PatternKind::Ctor {
                    path,
                    ctor,
                    payload,
                }
            }
            Tok::Kw(kw @ (Kw::True | Kw::False)) => {
                let t = self.bump();
                PatternKind::Ctor {
                    path: QName {
                        module: None,
                        name: Ident {
                            text: if kw == Kw::True { "True" } else { "False" }.to_string(),
                            span: t.span,
                        },
                    },
                    ctor: None,
                    payload: None,
                }
            }
            Tok::Minus | Tok::Int | Tok::Decimal | Tok::Float | Tok::Text => {
                let neg = self.eat(Tok::Minus);
                if neg && !matches!(self.tok().tok, Tok::Int | Tok::Decimal | Tok::Float) {
                    return Err(self.error_here("expected a number"));
                }
                PatternKind::Lit {
                    neg,
                    lit: self.lit()?,
                }
            }
            _ => return Err(self.error_here("expected a pattern")),
        };
        Ok(Pattern {
            kind,
            span: self.span_from(start),
        })
    }

    fn record_pat(&mut self) -> PResult<Vec<FieldPat>> {
        self.expect(Tok::LBrace, "`{`")?;
        let mut fields = Vec::new();
        if !self.at(Tok::RBrace) {
            loop {
                let field = self.field_word()?;
                let pattern = if self.eat(Tok::Colon) {
                    Some(self.pattern()?)
                } else {
                    None
                };
                fields.push(FieldPat { field, pattern });
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::RBrace, "`}`")?;
        Ok(fields)
    }

    fn fix_expr(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        self.expect_kw(Kw::Fix)?;
        let mut binds = Vec::new();
        loop {
            let name = self.lower("a name")?;
            self.expect(Tok::Eq, "`=`")?;
            binds.push((name, self.expr()?));
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect_kw(Kw::In)?;
        let body = self.expr()?;
        Ok(self.mk(ExprKind::Fix(binds, Box::new(body)), start))
    }

    fn lambda(&mut self) -> PResult<Expr> {
        let start = self.tok().span;
        self.expect_kw(Kw::Fn)?;
        let params = if self.at(Tok::LParen) {
            LambdaParams::List(self.params()?)
        } else {
            LambdaParams::One(self.lower("a parameter")?)
        };
        self.expect(Tok::Arrow, "`->`")?;
        let body = self.expr()?;
        Ok(self.mk(ExprKind::Lambda(params, Box::new(body)), start))
    }

    /// An index or unique field list, where a lone reserved word is a field name.
    pub(crate) fn index_fields(&mut self) -> PResult<Vec<Expr>> {
        self.expect(Tok::LParen, "`(`")?;
        let mut fields = Vec::new();
        loop {
            let t = self.tok();
            if matches!(t.tok, Tok::Kw(_)) && matches!(self.peek(1).tok, Tok::Comma | Tok::RParen) {
                self.bump();
                let name = self.ident(t);
                fields.push(Expr {
                    kind: ExprKind::Name(QName { module: None, name }),
                    span: t.span,
                });
            } else {
                fields.push(self.expr()?);
            }
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen, "`)`")?;
        Ok(fields)
    }

    pub(crate) fn key_fields(&mut self) -> PResult<Vec<Ident>> {
        if self.eat(Tok::LParen) {
            let mut fields = vec![self.field_word()?];
            while self.eat(Tok::Comma) {
                fields.push(self.field_word()?);
            }
            self.expect(Tok::RParen, "`)`")?;
            Ok(fields)
        } else {
            Ok(vec![self.field_word()?])
        }
    }
}

/// Binding levels of the infix operators and `not`, from loosest to tightest.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Union,
    Or,
    And,
    Not,
    Cmp,
    Add,
    Mul,
    Prefix,
}

impl Level {
    /// Returns the level of the right operand of a left-associative operator of this level.
    fn operand(self) -> Level {
        match self {
            Level::Union => Level::Or,
            Level::Or => Level::And,
            Level::And => Level::Not,
            Level::Not | Level::Cmp => Level::Add,
            Level::Add => Level::Mul,
            Level::Mul | Level::Prefix => Level::Prefix,
        }
    }
}
