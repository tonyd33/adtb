use super::{PResult, Parser};
use crate::ast::*;
use crate::lex::{Kw, Tok};

impl Parser<'_> {
    pub(crate) fn decl(&mut self) -> PResult<Decl> {
        match self.tok().tok {
            Tok::Kw(Kw::Type) => Ok(Decl::Type(self.type_decl()?)),
            Tok::Kw(Kw::Relation) => Ok(Decl::Relation(self.relation_decl()?)),
            Tok::Kw(Kw::Set) => Ok(Decl::Set(self.set_decl()?)),
            Tok::Kw(Kw::Fn) => Ok(Decl::Fn(self.fn_decl()?)),
            Tok::Kw(Kw::Constraint) => Ok(Decl::Constraint(self.constraint_decl()?)),
            Tok::Kw(Kw::Check) => Ok(Decl::Check(self.check_decl()?)),
            _ if self.at_word("store") => Ok(Decl::Store(self.store_decl()?)),
            _ if self.at_word("aggregate") => Ok(Decl::Aggregate(self.aggregate_decl()?)),
            _ => Err(self.error_here("expected a declaration")),
        }
    }

    pub(crate) fn at_fn_decl(&self) -> bool {
        self.at_kw(Kw::Fn)
            && self.peek(1).tok == Tok::Lower
            && matches!(self.peek(2).tok, Tok::LParen | Tok::LBracket)
    }

    pub(crate) fn type_decl(&mut self) -> PResult<TypeDecl> {
        let start = self.tok().span;
        self.expect_kw(Kw::Type)?;
        let name = self.upper("a type name")?;
        let mut params = Vec::new();
        while self.at(Tok::Lower) {
            let t = self.bump();
            params.push(self.ident(t));
        }
        let ordered = self.eat_kw(Kw::Ordered);
        let header = if self.at(Tok::LBrace) {
            Some(self.record_type()?)
        } else {
            None
        };
        self.expect(Tok::Eq, "`=`")?;
        let body = if self.at_word("new") {
            self.bump();
            let ty = self.app_type()?;
            let domain = if self.eat_kw(Kw::Where) {
                Some(self.expr()?)
            } else {
                None
            };
            TypeBody::New(ty, domain)
        } else if self.at(Tok::LBrace) {
            TypeBody::Record(self.record_type()?)
        } else {
            let mut ctors = vec![self.ctor_def()?];
            while self.eat(Tok::Bar) {
                ctors.push(self.ctor_def()?);
            }
            TypeBody::Ctors(ctors)
        };
        Ok(TypeDecl {
            name,
            params,
            ordered,
            header,
            body,
            span: self.span_from(start),
        })
    }

    pub(crate) fn ctor_def(&mut self) -> PResult<CtorDef> {
        let name = self.upper("a constructor name")?;
        let record = if self.at(Tok::LBrace) {
            Some(self.record_type()?)
        } else {
            None
        };
        Ok(CtorDef { name, record })
    }

    pub(crate) fn relation_decl(&mut self) -> PResult<RelationDecl> {
        let start = self.tok().span;
        self.expect_kw(Kw::Relation)?;
        let name = self.qlower("a relation name")?;
        self.expect(Tok::Colon, "`:`")?;
        let ty = self.app_type()?;
        let mut clauses = Vec::new();
        while let Some(c) = self.relation_clause(true)? {
            clauses.push(c);
        }
        Ok(RelationDecl {
            name,
            ty,
            clauses,
            span: self.span_from(start),
        })
    }

    pub(crate) fn generated(&mut self) -> PResult<Option<Ident>> {
        if self.eat_word("generate") {
            Ok(Some(self.lower("a generator name")?))
        } else {
            Ok(None)
        }
    }

    fn scope_after_in(&mut self) -> PResult<Option<CtorScope>> {
        if self.eat_kw(Kw::In) {
            Ok(Some(self.ctor_scope()?))
        } else {
            Ok(None)
        }
    }

    /// One clause of a relation (`full`) or a stored view, or `None` when none follows.
    fn relation_clause(&mut self, full: bool) -> PResult<Option<RelationClause>> {
        if self.eat_kw(Kw::Key) {
            let fields = self.key_fields()?;
            let generate = if full { self.generated()? } else { None };
            return Ok(Some(RelationClause::Key(fields, generate)));
        }
        if self.at_word("unique") && self.peek(1).tok == Tok::LParen {
            self.bump();
            let fields = self.index_fields()?;
            let scope = self.scope_after_in()?;
            return Ok(Some(RelationClause::Unique(fields, scope)));
        }
        let interval = self.at_word("interval") && self.peek_word(1, "index");
        if interval || (self.at_word("index") && self.peek(1).tok == Tok::LParen) {
            if interval {
                self.bump();
            }
            self.bump();
            let fields = self.index_fields()?;
            let scope = self.scope_after_in()?;
            return Ok(Some(RelationClause::Index {
                interval,
                fields,
                scope,
            }));
        }
        if !full {
            return Ok(None);
        }
        if self.eat_word("volatile") {
            return Ok(Some(RelationClause::Volatile));
        }
        if self.at_word("default")
            && matches!(self.peek(1).tok, Tok::Lower | Tok::Upper | Tok::Kw(_))
        {
            self.bump();
            let ctor = if self.at(Tok::Upper) {
                let c = self.upper("a constructor name")?;
                self.expect(Tok::Dot, "`.`")?;
                Some(c)
            } else {
                None
            };
            let field = self.field_word()?;
            self.expect(Tok::Eq, "`=`")?;
            let value = self.default_value()?;
            return Ok(Some(RelationClause::Default { ctor, field, value }));
        }
        if self.at_word("computed") && self.peek(1).tok == Tok::Lower {
            self.bump();
            let field = self.lower("a field name")?;
            self.expect(Tok::Colon, "`:`")?;
            let ty = self.ty()?;
            self.expect(Tok::Eq, "`=`")?;
            let e = self.expr()?;
            return Ok(Some(RelationClause::Computed(field, ty, e)));
        }
        Ok(None)
    }

    pub(crate) fn default_value(&mut self) -> PResult<DefaultValue> {
        if self.at_word("generate") && self.peek(1).tok == Tok::Lower {
            self.bump();
            Ok(DefaultValue::Generate(self.lower("a generator name")?))
        } else {
            Ok(DefaultValue::Expr(self.expr()?))
        }
    }

    pub(crate) fn set_decl(&mut self) -> PResult<SetDecl> {
        let start = self.tok().span;
        self.expect_kw(Kw::Set)?;
        let name = self.upper("a set name")?;
        self.expect_word("of")?;
        let relation = self.lower("a relation name")?;
        self.expect(Tok::Eq, "`=`")?;
        let mut members = vec![self.upper("a constructor name")?];
        while self.eat(Tok::Bar) {
            members.push(self.upper("a constructor name")?);
        }
        Ok(SetDecl {
            name,
            relation,
            members,
            span: self.span_from(start),
        })
    }

    pub(crate) fn fn_decl(&mut self) -> PResult<FnDecl> {
        let start = self.tok().span;
        self.expect_kw(Kw::Fn)?;
        let name = self.lower("a function name")?;
        let mut type_params = Vec::new();
        if self.eat(Tok::LBracket) {
            loop {
                let name = self.lower("a type parameter")?;
                let mut bounds = Vec::new();
                if self.eat(Tok::Colon) {
                    bounds.push(self.upper("a bound")?);
                    while self.eat(Tok::Plus) {
                        bounds.push(self.upper("a bound")?);
                    }
                }
                type_params.push(TypeParam { name, bounds });
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
            self.expect(Tok::RBracket, "`]`")?;
        }
        let params = self.params()?;
        self.expect(Tok::Arrow, "`->`")?;
        let result = self.ty()?;
        self.expect(Tok::Eq, "`=`")?;
        let body = if self.at_word("wasm") && self.peek(1).tok == Tok::Text {
            self.bump();
            let module = self.text_lit()?;
            self.expect_word("export")?;
            let export = self.text_lit()?;
            FnBody::Wasm { module, export }
        } else if self.at_word("host") && self.peek(1).tok == Tok::Text {
            self.bump();
            let name = self.text_lit()?;
            self.expect_word("version")?;
            let version = self.text_lit()?;
            let deterministic = self.eat_word("deterministic");
            FnBody::Host {
                name,
                version,
                deterministic,
            }
        } else {
            FnBody::Expr(self.expr()?)
        };
        Ok(FnDecl {
            name,
            type_params,
            params,
            result,
            body,
            span: self.span_from(start),
        })
    }

    fn limit(&mut self) -> PResult<Option<Expr>> {
        if self.at_word("limit") && self.peek(1).tok == Tok::Duration {
            self.bump();
            Ok(Some(self.prefix_e()?))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn query_decl(&mut self) -> PResult<QueryDecl> {
        let start = self.tok().span;
        self.expect_kw(Kw::Query)?;
        let name = self.lower("a query name")?;
        let params = self.params()?;
        let limit = self.limit()?;
        self.expect(Tok::Arrow, "`->`")?;
        let result = self.ty()?;
        self.expect(Tok::Eq, "`=`")?;
        let body = self.expr()?;
        Ok(QueryDecl {
            name,
            params,
            limit,
            result,
            body,
            span: self.span_from(start),
        })
    }

    pub(crate) fn txn_decl(&mut self) -> PResult<TxnDecl> {
        let start = self.tok().span;
        self.expect_kw(Kw::Txn)?;
        let name = self.lower("a transaction name")?;
        let params = self.params()?;
        let limit = self.limit()?;
        self.expect(Tok::Arrow, "`->`")?;
        let result = self.ty()?;
        self.expect(Tok::Eq, "`=`")?;
        let body = if self.at_elision() {
            let span = self.tok().span;
            self.bump();
            self.bump();
            self.bump();
            TransactBlock {
                options: Vec::new(),
                items: Vec::new(),
                ret: None,
                span,
            }
        } else {
            self.transact_block()?
        };
        Ok(TxnDecl {
            name,
            params,
            limit,
            result,
            body,
            span: self.span_from(start),
        })
    }

    pub(crate) fn constraint_decl(&mut self) -> PResult<ConstraintDecl> {
        let start = self.tok().span;
        self.expect_kw(Kw::Constraint)?;
        let name = self.lower("a constraint name")?;
        self.expect(Tok::Eq, "`=`")?;
        let query = self.expr()?;
        Ok(ConstraintDecl {
            name,
            query,
            span: self.span_from(start),
        })
    }

    pub(crate) fn check_decl(&mut self) -> PResult<CheckDecl> {
        let start = self.tok().span;
        self.expect_kw(Kw::Check)?;
        let name = self.lower("a check name")?;
        self.expect_word("on")?;
        let on = self.relation_ref()?;
        self.expect(Tok::LParen, "`(`")?;
        let predicate = self.expr()?;
        self.expect(Tok::RParen, "`)`")?;
        Ok(CheckDecl {
            name,
            on,
            predicate,
            span: self.span_from(start),
        })
    }

    pub(crate) fn store_decl(&mut self) -> PResult<StoreDecl> {
        let start = self.tok().span;
        self.expect_word("store")?;
        let view = self.qlower("a view name")?;
        let mut clauses = Vec::new();
        while let Some(c) = self.relation_clause(false)? {
            clauses.push(c);
        }
        Ok(StoreDecl {
            view,
            clauses,
            span: self.span_from(start),
        })
    }

    fn two_binders(&mut self) -> PResult<(Ident, Ident)> {
        self.expect(Tok::LParen, "`(`")?;
        let a = self.lower("a name")?;
        self.expect(Tok::Comma, "`,`")?;
        let b = self.lower("a name")?;
        self.expect(Tok::RParen, "`)`")?;
        self.expect(Tok::Arrow, "`->`")?;
        Ok((a, b))
    }

    pub(crate) fn aggregate_decl(&mut self) -> PResult<AggregateDecl> {
        let start = self.tok().span;
        self.expect_word("aggregate")?;
        let name = self.lower("an aggregate name")?;
        let params = self.params()?;
        self.expect(Tok::Arrow, "`->`")?;
        let result = self.ty()?;
        self.expect(Tok::Eq, "`=`")?;
        self.expect_word("state")?;
        let state = self.record_type()?;
        self.expect_word("init")?;
        let init = self.expr()?;
        self.expect_word("step")?;
        let (s, x) = self.two_binders()?;
        let step = (s, x, self.expr()?);
        self.expect_word("merge")?;
        let (a, b) = self.two_binders()?;
        let merge = (a, b, self.expr()?);
        self.expect_word("finish")?;
        let f = self.lower("a name")?;
        self.expect(Tok::Arrow, "`->`")?;
        let finish = (f, self.expr()?);
        Ok(AggregateDecl {
            name,
            params,
            result,
            state,
            init,
            step,
            merge,
            finish,
            span: self.span_from(start),
        })
    }

    pub(crate) fn bundle_header(&mut self) -> PResult<BundleHeader> {
        let start = self.tok().span;
        self.expect_kw(Kw::Bundle)?;
        let name = self.bundle_name()?;
        self.expect_word("pins")?;
        let pins = self.version()?;
        let catalog = if self.eat_word("catalog") {
            Some(self.version()?)
        } else {
            None
        };
        self.expect_word("edition")?;
        let edition = self.text_lit()?;
        let mut clauses = Vec::new();
        loop {
            if self.at_word("lease") && self.peek(1).tok == Tok::Duration {
                self.bump();
                clauses.push(HeaderClause::Lease(self.prefix_e()?));
            } else if self.at_word("priority") && self.peek(1).tok == Tok::Lower {
                self.bump();
                clauses.push(HeaderClause::Priority(self.lower("a priority")?));
            } else if self.at_word("budget") && self.peek(1).tok == Tok::LBrace {
                self.bump();
                clauses.push(HeaderClause::Budget(self.record_expr()?));
            } else if self.at_word("hide") {
                self.bump();
                self.expect_word("new")?;
                self.expect_word("constructors")?;
                clauses.push(HeaderClause::HideNewConstructors);
            } else {
                break;
            }
        }
        Ok(BundleHeader {
            name,
            pins,
            catalog,
            edition,
            clauses,
            span: self.span_from(start),
        })
    }

    pub(crate) fn import(&mut self) -> PResult<Import> {
        self.expect_kw(Kw::Import)?;
        if self.eat_kw(Kw::Fn) {
            Ok(Import::Fn(self.qlower("a function name")?))
        } else {
            Ok(Import::Module(self.lower("a module name")?))
        }
    }

    pub(crate) fn bundle_item(&mut self) -> PResult<BundleItem> {
        match self.tok().tok {
            Tok::Kw(Kw::Fn) => Ok(BundleItem::Fn(self.fn_decl()?)),
            Tok::Kw(Kw::Query) => Ok(BundleItem::Query(self.query_decl()?)),
            Tok::Kw(Kw::Txn) => Ok(BundleItem::Txn(self.txn_decl()?)),
            _ if self.at_word("aggregate") => Ok(BundleItem::Aggregate(self.aggregate_decl()?)),
            _ if self.at_word("feed") => {
                let start = self.tok().span;
                self.bump();
                let name = self.lower("a feed name")?;
                self.expect_word("of")?;
                let of = self.relation_ref()?;
                Ok(BundleItem::Feed(FeedDecl {
                    name,
                    of,
                    span: self.span_from(start),
                }))
            }
            _ => Err(self.error_here("expected `fn`, `query`, `txn`, `feed` or `aggregate`")),
        }
    }
}
