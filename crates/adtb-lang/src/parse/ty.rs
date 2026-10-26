use super::{PResult, Parser};
use crate::ast::*;
use crate::lex::{Kw, Tok};
use crate::types::is_prelude_type;

/// Words that end a type because a clause follows it, so they are never type arguments.
const CLAUSE_WORDS: &[&str] = &[
    "on", "volatile", "unique", "interval", "index", "default", "computed", "fill", "generate",
];

impl Parser<'_> {
    pub(crate) fn ty(&mut self) -> PResult<Type> {
        self.nested(Self::fn_type)
    }

    fn fn_type(&mut self) -> PResult<Type> {
        let start = self.tok().span;
        let lhs = self.sum_type()?;
        if self.eat(Tok::Arrow) {
            let rhs = self.ty()?;
            return Ok(Type {
                kind: TypeKind::Fn(Box::new(lhs), Box::new(rhs)),
                span: self.span_from(start),
            });
        }
        Ok(lhs)
    }

    fn at_anon_ctor(&self) -> bool {
        self.at(Tok::Upper)
            && !is_prelude_type(self.text_of(self.tok()))
            && matches!(self.peek(1).tok, Tok::LBrace | Tok::Bar)
    }

    fn sum_type(&mut self) -> PResult<Type> {
        let start = self.tok().span;
        if !self.at_anon_ctor() {
            let t = self.app_type()?;
            if let TypeKind::Named { name, .. } = &t.kind
                && self.at(Tok::Bar)
                && is_prelude_type(&name.name.text)
            {
                return Err(self.prelude_ctor(&name.name));
            }
            return Ok(t);
        }
        let mut ctors = vec![self.anon_ctor()?];
        while self.eat(Tok::Bar) {
            ctors.push(self.anon_ctor()?);
        }
        Ok(Type {
            kind: TypeKind::Sum(ctors),
            span: self.span_from(start),
        })
    }

    fn anon_ctor(&mut self) -> PResult<AnonCtor> {
        let name = self.upper("a constructor name")?;
        if is_prelude_type(&name.text) {
            return Err(self.prelude_ctor(&name));
        }
        let record = if self.at(Tok::LBrace) {
            Some(self.record_type()?)
        } else {
            None
        };
        Ok(AnonCtor { name, record })
    }

    fn prelude_ctor(&mut self, name: &Ident) -> super::Fail {
        self.error_at(
            name.span,
            format!(
                "`{}` is a prelude type and cannot name an anonymous constructor",
                name.text
            ),
        )
    }

    fn at_type_arg(&self) -> bool {
        let next = self.peek(1);
        let called = next.tok == Tok::LParen && !next.spaced;
        match self.tok().tok {
            Tok::LParen | Tok::Int | Tok::LBrace => true,
            Tok::Upper => !called,
            Tok::Lower => {
                !called
                    && !CLAUSE_WORDS.contains(&self.text_of(self.tok()))
                    && !self.at_item_start()
            }
            _ => false,
        }
    }

    pub(crate) fn app_type(&mut self) -> PResult<Type> {
        let start = self.tok().span;
        let mut atom = self.type_atom()?;
        if let TypeKind::Named {
            name,
            sel: None,
            args,
        } = &mut atom.kind
        {
            if self.at(Tok::LBrace) && is_prelude_type(&name.name.text) {
                args.push(TypeArg::Type(self.type_atom()?));
            }
            while self.at_type_arg() {
                if self.at(Tok::Int) {
                    let t = self.tok();
                    let n = self.int_lit()?;
                    args.push(TypeArg::Int(n, t.span));
                } else if self.at(Tok::LBrace) {
                    return Err(self.error_here("a record type argument is written in parentheses"));
                } else {
                    args.push(TypeArg::Type(self.type_atom()?));
                }
            }
            atom.span = self.span_from(start);
        }
        Ok(atom)
    }

    fn type_atom(&mut self) -> PResult<Type> {
        let start = self.tok().span;
        let kind = match self.tok().tok {
            Tok::Upper if self.text_of(self.tok()) == "Key" => {
                let word = self.bump().span;
                TypeKind::Key(word, self.relation_ref()?)
            }
            Tok::Upper => self.named_type()?,
            Tok::Lower if self.at_qupper() => self.named_type()?,
            Tok::Lower => {
                let t = self.bump();
                TypeKind::Var(self.ident(t))
            }
            Tok::LParen if self.peek(1).tok == Tok::RParen => {
                self.bump();
                self.bump();
                TypeKind::Unit
            }
            Tok::LParen => {
                self.bump();
                let inner = self.ty()?;
                self.expect(Tok::RParen, "`)`")?;
                return Ok(Type {
                    kind: inner.kind,
                    span: self.span_from(start),
                });
            }
            Tok::LBrace => TypeKind::Record(self.record_type()?),
            _ => return Err(self.error_here("expected a type")),
        };
        Ok(Type {
            kind,
            span: self.span_from(start),
        })
    }

    fn named_type(&mut self) -> PResult<TypeKind> {
        let name = self.qupper("a type name")?;
        let sel = if self.at_ctor_sel_after_dot() {
            self.bump();
            Some(self.ctor_sel()?)
        } else {
            None
        };
        Ok(TypeKind::Named {
            name,
            sel,
            args: Vec::new(),
        })
    }

    pub(crate) fn record_type(&mut self) -> PResult<Vec<FieldType>> {
        self.expect(Tok::LBrace, "`{`")?;
        let mut fields = Vec::new();
        if !self.at(Tok::RBrace) {
            loop {
                fields.push(self.field_type()?);
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::RBrace, "`}`")?;
        Ok(fields)
    }

    fn field_type(&mut self) -> PResult<FieldType> {
        let name = self.field_word()?;
        self.expect(Tok::Colon, "`:`")?;
        let ty = self.ty()?;
        let on_delete = if self.at_word("on") && self.peek(1).tok == Tok::Kw(Kw::Delete) {
            self.bump();
            self.bump();
            Some(self.ref_action()?)
        } else {
            None
        };
        Ok(FieldType {
            name,
            ty,
            on_delete,
        })
    }

    fn ref_action(&mut self) -> PResult<RefAction> {
        if self.eat_word("restrict") {
            Ok(RefAction::Restrict)
        } else if self.eat_word("cascade") {
            Ok(RefAction::Cascade)
        } else if self.eat_word("clear") {
            Ok(RefAction::Clear)
        } else if self.eat_word("retag") {
            let ctor = self.upper("a constructor name")?;
            let record = if self.at(Tok::LBrace) {
                Some(self.record_expr()?)
            } else {
                None
            };
            Ok(RefAction::Retag(ctor, record))
        } else {
            Err(self.error_here("expected `restrict`, `cascade`, `clear` or `retag`"))
        }
    }

    pub(crate) fn params(&mut self) -> PResult<Vec<Param>> {
        self.expect(Tok::LParen, "`(`")?;
        let mut params = Vec::new();
        if !self.at(Tok::RParen) {
            loop {
                let name = self.lower("a parameter name")?;
                self.expect(Tok::Colon, "`:`")?;
                let ty = self.ty()?;
                params.push(Param { name, ty });
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::RParen, "`)`")?;
        Ok(params)
    }
}
