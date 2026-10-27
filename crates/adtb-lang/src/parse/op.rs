use super::{PResult, Parser};
use crate::ast::*;
use crate::lex::{Kw, Tok};

impl Parser<'_> {
    /// The current token starts a schema operator, judged by its verb and the word after it.
    pub(crate) fn at_operator(&self) -> bool {
        let t = self.tok();
        if t.tok == Tok::Lower && self.text_of(t) == "store" {
            return self.peek(1).tok == Tok::Lower;
        }
        if t.tok != Tok::Lower {
            return false;
        }
        let next = self.peek(1);
        let next_word = match next.tok {
            Tok::Lower => Some(self.text_of(next)),
            Tok::Kw(kw) => Some(match kw {
                Kw::Module => "module",
                Kw::Type => "type",
                Kw::Relation => "relation",
                Kw::Set => "set",
                Kw::Fn => "fn",
                Kw::Constraint => "constraint",
                Kw::Check => "check",
                _ => "",
            }),
            _ => None,
        };
        match self.text_of(t) {
            "add" => matches!(
                next_word,
                Some(
                    "module"
                        | "type"
                        | "relation"
                        | "set"
                        | "constructor"
                        | "field"
                        | "unique"
                        | "index"
                        | "interval"
                        | "default"
                        | "constraint"
                        | "check"
                        | "computed"
                )
            ),
            "drop" => matches!(
                next_word,
                Some(
                    "module"
                        | "type"
                        | "relation"
                        | "set"
                        | "field"
                        | "unique"
                        | "index"
                        | "interval"
                        | "default"
                        | "fn"
                        | "constraint"
                        | "check"
                        | "computed"
                        | "aggregate"
                )
            ),
            "rename" => matches!(
                next_word,
                Some("type" | "relation" | "set" | "constructor" | "field")
            ),
            "remove" | "split" | "nest" | "unnest" | "merge" => next_word == Some("constructor"),
            "move" | "alter" => next_word == Some("field"),
            "rekey" => next_word == Some("relation"),
            "define" => matches!(next_word, Some("fn" | "aggregate")),
            "redefine" => matches!(next_word, Some("fn" | "computed")),
            "unstore" | "uninstall" | "install" => next.tok == Tok::Lower,
            "enable" | "disable" | "deprecate" => {
                matches!(next.tok, Tok::Lower | Tok::Upper)
            }
            "revert" => next.tok == Tok::Int,
            "script" => matches!(
                next.tok,
                Tok::Kw(
                    Kw::Insert | Kw::Update | Kw::Delete | Kw::Fail | Kw::Skip | Kw::If | Kw::Match
                ) | Tok::LParen
            ),
            "adopt" => next_word == Some("tzdb"),
            "grant" | "revoke" => true,
            _ => false,
        }
    }

    pub(crate) fn operator(&mut self) -> PResult<Operator> {
        let start = self.tok().span;
        let kind = self.operator_kind()?;
        Ok(Operator {
            kind,
            span: self.span_from(start),
        })
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one arm per operator verb; each operator would need its own parser"
    )]
    fn operator_kind(&mut self) -> PResult<OperatorKind> {
        if self.at_word("store") {
            return Ok(OperatorKind::Store(self.store_decl()?));
        }
        if self.at_word("grant") {
            return Ok(OperatorKind::Grant(self.grant()?));
        }
        if self.at_word("revoke") {
            return Ok(OperatorKind::Revoke(self.revoke()?));
        }
        let verb = self.lower("a schema operator")?;
        match verb.text.as_str() {
            "add" => self.add_op(),
            "drop" => self.drop_op(),
            "rename" => self.rename_op(),
            "remove" => {
                self.expect_word("constructor")?;
                Ok(OperatorKind::RemoveConstructor(self.ctor_path()?))
            }
            "split" => {
                self.expect_word("constructor")?;
                let path = self.ctor_path()?;
                self.expect_kw(Kw::Into)?;
                let mut into = vec![self.ctor_def()?];
                while self.eat(Tok::Bar) {
                    into.push(self.ctor_def()?);
                }
                self.expect_kw(Kw::By)?;
                self.expect_kw(Kw::Match)?;
                let arms = self.arms()?;
                let mut clauses = Vec::new();
                loop {
                    if self.at_word("unique") && self.peek(1).tok == Tok::LParen {
                        self.bump();
                        let fields = self.index_fields()?;
                        self.expect_kw(Kw::In)?;
                        clauses.push(SplitClause::Unique(fields, self.ctor_scope()?));
                    } else if self.at_word("index") || self.at_word("interval") {
                        let interval = self.eat_word("interval");
                        self.expect_word("index")?;
                        let fields = self.index_fields()?;
                        self.expect_kw(Kw::In)?;
                        let scope = self.ctor_scope()?;
                        clauses.push(SplitClause::Index {
                            interval,
                            fields,
                            scope,
                        });
                    } else if let Some(arm) = self.arm_clause()? {
                        clauses.push(SplitClause::Arm(arm));
                    } else {
                        break;
                    }
                }
                Ok(OperatorKind::SplitConstructor {
                    path,
                    into,
                    arms,
                    clauses,
                })
            }
            "merge" => {
                self.expect_word("constructor")?;
                let (ty, sel) = self.ctor_set_path()?;
                self.expect_kw(Kw::Into)?;
                let into = self.ctor_def()?;
                let arms = if self.eat_kw(Kw::By) {
                    self.expect_kw(Kw::Match)?;
                    Some(self.arms()?)
                } else {
                    None
                };
                let clauses = self.arm_clauses()?;
                Ok(OperatorKind::MergeConstructor {
                    ty,
                    sel,
                    into,
                    arms,
                    clauses,
                })
            }
            "nest" => {
                self.expect_word("constructor")?;
                let (ty, sel) = self.ctor_set_path()?;
                self.expect_kw(Kw::Into)?;
                let into = self.ctor_def()?;
                let clauses = self.arm_clauses()?;
                Ok(OperatorKind::NestConstructor {
                    ty,
                    sel,
                    into,
                    clauses,
                })
            }
            "unnest" => {
                self.expect_word("constructor")?;
                let path = self.ctor_path()?;
                let clauses = self.arm_clauses()?;
                Ok(OperatorKind::UnnestConstructor(path, clauses))
            }
            "alter" => {
                self.expect_word("field")?;
                let path = self.field_path()?;
                if !(self.at_word("on") && self.peek(1).tok == Tok::Kw(Kw::Delete)) {
                    return Err(self.error_here("expected `on delete`"));
                }
                self.bump();
                self.bump();
                let start = self.tok().span;
                let action = self.ref_action()?;
                Ok(OperatorKind::AlterField {
                    path,
                    action,
                    span: self.span_from(start),
                })
            }
            "move" => {
                self.expect_word("field")?;
                let path = self.field_path()?;
                self.expect_word("to")?;
                let to = self.qupper("a type name")?;
                let sel = if self.at_ctor_sel_after_dot() {
                    self.bump();
                    Some(self.ctor_sel()?)
                } else {
                    None
                };
                let fill = if self.eat_word("fill") {
                    Some(self.fill()?)
                } else {
                    None
                };
                Ok(OperatorKind::MoveField {
                    path,
                    to,
                    sel,
                    fill,
                })
            }
            "rekey" => {
                self.expect_kw(Kw::Relation)?;
                let relation = self.qlower("a relation name")?;
                self.expect_kw(Kw::Key)?;
                let key = self.key_fields()?;
                let mut carry = Vec::new();
                if self.eat_word("carry") {
                    carry.push(self.field_path()?);
                    while self.eat(Tok::Comma) {
                        carry.push(self.field_path()?);
                    }
                }
                Ok(OperatorKind::Rekey {
                    relation,
                    key,
                    carry,
                })
            }
            "define" => {
                if self.at_word("aggregate") {
                    Ok(OperatorKind::DefineAggregate(self.aggregate_decl()?))
                } else {
                    Ok(OperatorKind::DefineFn(self.fn_decl()?))
                }
            }
            "redefine" => {
                if self.eat_word("computed") {
                    let (relation, field, ty, expr) = self.computed_def()?;
                    Ok(OperatorKind::RedefineComputed {
                        relation,
                        field,
                        ty,
                        expr,
                    })
                } else {
                    Ok(OperatorKind::RedefineFn(self.fn_decl()?))
                }
            }
            "unstore" => Ok(OperatorKind::Unstore(self.qlower("a view name")?)),
            "enable" => Ok(OperatorKind::Enable(self.ctor_target()?)),
            "disable" => Ok(OperatorKind::Disable(self.ctor_target()?)),
            "deprecate" => Ok(OperatorKind::Deprecate(self.ctor_target()?)),
            "revert" => {
                let t = self.expect(Tok::Int, "a migration number")?;
                let migration = self.text_of(t).to_string();
                if migration.len() < 4 {
                    return Err(
                        self.error_at(t.span, "a migration number has at least four digits")
                    );
                }
                let mut fills = Vec::new();
                while self.eat_word("fill") {
                    let path = self.field_path()?;
                    self.expect(Tok::Eq, "`=`")?;
                    fills.push((path, self.fill()?));
                }
                Ok(OperatorKind::Revert { migration, fills })
            }
            "script" => Ok(OperatorKind::Script(self.write_expr()?)),
            "install" => {
                let name = self.lower("an extension name")?;
                let version = self.text_lit()?;
                self.expect_kw(Kw::From)?;
                let from = self.text_lit()?;
                Ok(OperatorKind::Install {
                    name,
                    version,
                    from,
                })
            }
            "uninstall" => Ok(OperatorKind::Uninstall(self.lower("an extension name")?)),
            "adopt" => {
                self.expect_word("tzdb")?;
                let release = self.text_lit()?;
                self.expect_kw(Kw::From)?;
                let from = self.text_lit()?;
                Ok(OperatorKind::AdoptTzdb { release, from })
            }
            _ => Err(self.error_at(verb.span, "expected a schema operator")),
        }
    }

    fn add_op(&mut self) -> PResult<OperatorKind> {
        match self.tok().tok {
            Tok::Kw(Kw::Module) => {
                self.bump();
                Ok(OperatorKind::AddModule(self.lower("a module name")?))
            }
            Tok::Kw(Kw::Type) => Ok(OperatorKind::AddType(self.type_decl()?)),
            Tok::Kw(Kw::Relation) => Ok(OperatorKind::AddRelation(self.relation_decl()?)),
            Tok::Kw(Kw::Set) => Ok(OperatorKind::AddSet(self.set_decl()?)),
            Tok::Kw(Kw::Constraint) => Ok(OperatorKind::AddConstraint(self.constraint_decl()?)),
            Tok::Kw(Kw::Check) => Ok(OperatorKind::AddCheck(self.check_decl()?)),
            _ if self.eat_word("constructor") => {
                let path = self.ctor_path()?;
                let record = if self.at(Tok::LBrace) {
                    Some(self.record_type()?)
                } else {
                    None
                };
                let place = if self.eat_word("before") {
                    Some(CtorPlace::Before(self.upper("a constructor name")?))
                } else if self.eat_word("after") {
                    Some(CtorPlace::After(self.upper("a constructor name")?))
                } else {
                    None
                };
                let mut joins = Vec::new();
                let mut arms = Vec::new();
                loop {
                    if self.eat_word("joins") {
                        joins.push(self.relation_ref()?);
                        while self.eat(Tok::Comma) {
                            joins.push(self.relation_ref()?);
                        }
                    } else if let Some(arm) = self.arm_clause()? {
                        arms.push(arm);
                    } else {
                        break;
                    }
                }
                Ok(OperatorKind::AddConstructor {
                    path,
                    record,
                    place,
                    joins,
                    arms,
                })
            }
            _ if self.eat_word("field") => {
                let path = self.field_path()?;
                self.expect(Tok::Colon, "`:`")?;
                let ty = self.ty()?;
                let on_delete = self.on_delete()?.map(|a| a.action);
                let decl = self.span_from(path.field.span);
                self.expect_word("fill")?;
                let fill = self.fill()?;
                Ok(OperatorKind::AddField {
                    path,
                    ty,
                    on_delete,
                    decl,
                    fill,
                })
            }
            _ if self.eat_word("unique") => {
                let fields = self.index_fields()?;
                self.expect_kw(Kw::In)?;
                Ok(OperatorKind::AddUnique(fields, self.relation_ref()?))
            }
            _ if self.at_word("index") || self.at_word("interval") => {
                let interval = self.eat_word("interval");
                self.expect_word("index")?;
                let fields = self.index_fields()?;
                self.expect_kw(Kw::In)?;
                let on = self.relation_ref()?;
                Ok(OperatorKind::AddIndex {
                    interval,
                    fields,
                    on,
                })
            }
            _ if self.eat_word("default") => {
                let (relation, ctor, field) = self.default_path()?;
                self.expect(Tok::Eq, "`=`")?;
                let value = self.default_value()?;
                Ok(OperatorKind::AddDefault {
                    relation,
                    ctor,
                    field,
                    value,
                })
            }
            _ if self.eat_word("computed") => {
                let (relation, field, ty, expr) = self.computed_def()?;
                Ok(OperatorKind::AddComputed {
                    relation,
                    field,
                    ty,
                    expr,
                })
            }
            _ => Err(self.error_here("expected what to add")),
        }
    }

    fn drop_op(&mut self) -> PResult<OperatorKind> {
        match self.tok().tok {
            Tok::Kw(Kw::Module) => {
                self.bump();
                Ok(OperatorKind::DropModule(self.lower("a module name")?))
            }
            Tok::Kw(Kw::Type) => {
                self.bump();
                Ok(OperatorKind::DropType(self.qupper("a type name")?))
            }
            Tok::Kw(Kw::Relation) => {
                self.bump();
                Ok(OperatorKind::DropRelation(self.qlower("a relation name")?))
            }
            Tok::Kw(Kw::Set) => {
                self.bump();
                let set = self.upper("a set name")?;
                self.expect_word("of")?;
                Ok(OperatorKind::DropSet(set, self.qlower("a relation name")?))
            }
            Tok::Kw(Kw::Fn) => {
                self.bump();
                Ok(OperatorKind::DropFn(self.qlower("a function name")?))
            }
            Tok::Kw(Kw::Constraint) => {
                self.bump();
                Ok(OperatorKind::DropConstraint(
                    self.qlower("a constraint name")?,
                ))
            }
            Tok::Kw(Kw::Check) => {
                self.bump();
                Ok(OperatorKind::DropCheck(self.qlower("a check name")?))
            }
            _ if self.eat_word("field") => Ok(OperatorKind::DropField(self.field_path()?)),
            _ if self.eat_word("unique") => {
                let fields = self.index_fields()?;
                self.expect_kw(Kw::In)?;
                Ok(OperatorKind::DropUnique(fields, self.relation_ref()?))
            }
            _ if self.at_word("index") || self.at_word("interval") => {
                let interval = self.eat_word("interval");
                self.expect_word("index")?;
                let fields = self.index_fields()?;
                self.expect_kw(Kw::In)?;
                let on = self.relation_ref()?;
                Ok(OperatorKind::DropIndex {
                    interval,
                    fields,
                    on,
                })
            }
            _ if self.eat_word("default") => {
                let (relation, ctor, field) = self.default_path()?;
                Ok(OperatorKind::DropDefault {
                    relation,
                    ctor,
                    field,
                })
            }
            _ if self.eat_word("computed") => {
                let relation = self.qlower("a relation name")?;
                self.expect(Tok::Dot, "`.`")?;
                Ok(OperatorKind::DropComputed(
                    relation,
                    self.lower("a field name")?,
                ))
            }
            _ if self.eat_word("aggregate") => Ok(OperatorKind::DropAggregate(
                self.qlower("an aggregate name")?,
            )),
            _ => Err(self.error_here("expected what to drop")),
        }
    }

    fn rename_op(&mut self) -> PResult<OperatorKind> {
        match self.tok().tok {
            Tok::Kw(Kw::Type) => {
                self.bump();
                let from = self.qupper("a type name")?;
                self.expect_word("to")?;
                Ok(OperatorKind::RenameType(from, self.upper("a type name")?))
            }
            Tok::Kw(Kw::Relation) => {
                self.bump();
                let from = self.qlower("a relation name")?;
                self.expect_word("to")?;
                Ok(OperatorKind::RenameRelation(
                    from,
                    self.lower("a relation name")?,
                ))
            }
            Tok::Kw(Kw::Set) => {
                self.bump();
                let set = self.upper("a set name")?;
                self.expect_word("of")?;
                let relation = self.qlower("a relation name")?;
                self.expect_word("to")?;
                Ok(OperatorKind::RenameSet(
                    set,
                    relation,
                    self.upper("a set name")?,
                ))
            }
            _ if self.eat_word("constructor") => {
                let path = self.ctor_path()?;
                self.expect_word("to")?;
                Ok(OperatorKind::RenameConstructor(
                    path,
                    self.upper("a constructor name")?,
                ))
            }
            _ if self.eat_word("field") => {
                let path = self.field_path()?;
                self.expect_word("to")?;
                Ok(OperatorKind::RenameField(path, self.field_word()?))
            }
            _ => Err(self.error_here("expected what to rename")),
        }
    }

    fn ctor_set_path(&mut self) -> PResult<(QName, CtorSel)> {
        let ty = self.qupper("a type name")?;
        self.expect(Tok::Dot, "`.`")?;
        let sel = self.ctor_sel()?;
        Ok((ty, sel))
    }

    fn field_path(&mut self) -> PResult<FieldPath> {
        let ty = self.qupper("a type name")?;
        self.expect(Tok::Dot, "`.`")?;
        let ctor = if self.at(Tok::Upper) {
            let c = self.upper("a constructor name")?;
            self.expect(Tok::Dot, "`.`")?;
            Some(c)
        } else {
            None
        };
        let field = self.field_word()?;
        Ok(FieldPath { ty, ctor, field })
    }

    fn default_path(&mut self) -> PResult<(QName, Option<Ident>, Ident)> {
        let relation = self.qlower("a relation name")?;
        self.expect(Tok::Dot, "`.`")?;
        let ctor = if self.at(Tok::Upper) {
            let c = self.upper("a constructor name")?;
            self.expect(Tok::Dot, "`.`")?;
            Some(c)
        } else {
            None
        };
        let field = self.field_word()?;
        Ok((relation, ctor, field))
    }

    fn computed_def(&mut self) -> PResult<(QName, Ident, Type, Expr)> {
        let relation = self.qlower("a relation name")?;
        self.expect(Tok::Dot, "`.`")?;
        let field = self.lower("a field name")?;
        self.expect(Tok::Colon, "`:`")?;
        let ty = self.ty()?;
        self.expect(Tok::Eq, "`=`")?;
        let expr = self.expr()?;
        Ok((relation, field, ty, expr))
    }

    fn arm_clause(&mut self) -> PResult<Option<ArmClause>> {
        if !(self.at_word("arm") && self.peek(1).tok == Tok::Lower) {
            return Ok(None);
        }
        self.bump();
        let target = self.qlower("a dependent name")?;
        let field = if self.eat(Tok::Dot) {
            Some(self.lower("a computed field name")?)
        } else {
            None
        };
        self.expect(Tok::Colon, "`:`")?;
        let pattern = self.pattern()?;
        self.expect(Tok::Arrow, "`->`")?;
        let body = self.expr()?;
        Ok(Some(ArmClause {
            target,
            field,
            pattern,
            body,
        }))
    }

    fn arm_clauses(&mut self) -> PResult<Vec<ArmClause>> {
        let mut out = Vec::new();
        while let Some(a) = self.arm_clause()? {
            out.push(a);
        }
        Ok(out)
    }

    fn fill(&mut self) -> PResult<Fill> {
        if self.at_kw(Kw::Match) && self.peek(1).tok == Tok::Bar {
            self.bump();
            Ok(Fill::Match(self.arms()?))
        } else {
            Ok(Fill::Expr(self.expr()?))
        }
    }

    fn ctor_target(&mut self) -> PResult<CtorTarget> {
        if self.at_qupper() {
            Ok(CtorTarget::Type(self.ctor_path()?))
        } else {
            Ok(CtorTarget::Relation(self.relation_ref()?))
        }
    }

    pub(crate) fn grant(&mut self) -> PResult<Grant> {
        let start = self.tok().span;
        self.expect_word("grant")?;
        let privileges = self.privileges()?;
        self.expect_word("to")?;
        let to = self.lower("a role name")?;
        let grant_option = if self.eat_kw(Kw::With) {
            self.expect_word("grant")?;
            self.expect_word("option")?;
            true
        } else {
            false
        };
        Ok(Grant {
            privileges,
            to,
            grant_option,
            span: self.span_from(start),
        })
    }

    pub(crate) fn revoke(&mut self) -> PResult<Revoke> {
        let start = self.tok().span;
        self.expect_word("revoke")?;
        let privileges = self.privileges()?;
        self.expect_kw(Kw::From)?;
        let from = self.lower("a role name")?;
        Ok(Revoke {
            privileges,
            from,
            span: self.span_from(start),
        })
    }

    fn privileges(&mut self) -> PResult<Vec<Privilege>> {
        let mut out = vec![self.privilege()?];
        while self.eat(Tok::Comma) {
            out.push(self.privilege()?);
        }
        Ok(out)
    }

    fn field_list(&mut self) -> PResult<Option<Vec<Ident>>> {
        if !self.at(Tok::LParen) {
            return Ok(None);
        }
        self.bump();
        let mut fields = vec![self.field_word()?];
        while self.eat(Tok::Comma) {
            fields.push(self.field_word()?);
        }
        self.expect(Tok::RParen, "`)`")?;
        Ok(Some(fields))
    }

    fn privilege(&mut self) -> PResult<Privilege> {
        match self.tok().tok {
            Tok::Kw(Kw::Insert) => {
                self.bump();
                Ok(Privilege::Insert(self.relation_ref()?))
            }
            Tok::Kw(Kw::Update) => {
                self.bump();
                let r = self.relation_ref()?;
                Ok(Privilege::Update(r, self.field_list()?))
            }
            Tok::Kw(Kw::Delete) => {
                self.bump();
                Ok(Privilege::Delete(self.relation_ref()?))
            }
            Tok::Kw(Kw::Schema) => {
                let t = self.bump();
                Ok(Privilege::Capability(self.ident(t)))
            }
            Tok::Lower => {
                let t = self.tok();
                match self.text_of(t) {
                    "read" => {
                        self.bump();
                        let r = self.relation_ref()?;
                        Ok(Privilege::Read(r, self.field_list()?))
                    }
                    "retag" => {
                        self.bump();
                        let from = self.relation_ref()?;
                        self.expect(Tok::Arrow, "`->`")?;
                        Ok(Privilege::Retag(from, self.relation_ref()?))
                    }
                    "call" => {
                        self.bump();
                        if self.eat_kw(Kw::Fn) {
                            return Ok(Privilege::CallFn(self.qlower("a function name")?));
                        }
                        let target = self.call_target()?;
                        let query = if self.eat(Tok::ColonColon) {
                            Some(self.lower("a query name")?)
                        } else {
                            None
                        };
                        Ok(Privilege::Call(target, query))
                    }
                    "adhoc" | "assert" | "monitor" | "cancel" | "maintain" | "configure"
                    | "read_files" | "write_files" => {
                        self.bump();
                        Ok(Privilege::Capability(self.ident(t)))
                    }
                    _ => Err(self.error_here("expected a privilege")),
                }
            }
            _ => Err(self.error_here("expected a privilege")),
        }
    }
}
