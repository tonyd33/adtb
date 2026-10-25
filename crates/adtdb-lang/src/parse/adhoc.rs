use super::{PResult, Parser};
use crate::ast::*;
use crate::lex::{Kw, Tok};

impl Parser<'_> {
    pub(crate) fn adhoc_item(&mut self) -> PResult<AdhocItem> {
        let start = self.tok().span;
        match self.tok().tok {
            Tok::Kw(Kw::Transact) => return Ok(AdhocItem::Transact(self.transact_block()?)),
            Tok::Kw(Kw::Insert | Kw::Update | Kw::Delete | Kw::Fail | Kw::Skip) => {
                return Ok(AdhocItem::Write(self.write_expr()?));
            }
            Tok::Kw(Kw::If | Kw::Match) | Tok::LParen => {
                if let Some(w) = self.attempt(|p| p.write_expr()) {
                    return Ok(AdhocItem::Write(w));
                }
                return Ok(AdhocItem::Expr(self.expr()?));
            }
            Tok::Kw(Kw::Temp) => return Ok(AdhocItem::Temp(self.temp_decl()?)),
            Tok::Kw(Kw::Set) if self.peek_word(1, "setting") => {
                self.bump();
                self.bump();
                let name = self.upper("a setting name")?;
                self.expect(Tok::Eq, "`=`")?;
                let value = self.expr()?;
                return Ok(AdhocItem::Command(
                    Command::SetSetting(name, value),
                    self.span_from(start),
                ));
            }
            _ => {}
        }
        if self.at_word("grant") {
            return Ok(AdhocItem::Grant(self.grant()?));
        }
        if self.at_word("revoke") {
            return Ok(AdhocItem::Revoke(self.revoke()?));
        }
        if let Some(stmt) = self.role_stmt()? {
            return Ok(AdhocItem::Role(stmt, self.span_from(start)));
        }
        if let Some(stmt) = self.peer_stmt()? {
            return Ok(AdhocItem::Peer(stmt, self.span_from(start)));
        }
        if let Some(cmd) = self.command()? {
            return Ok(AdhocItem::Command(cmd, self.span_from(start)));
        }
        Ok(AdhocItem::Expr(self.expr()?))
    }

    fn temp_decl(&mut self) -> PResult<TempDecl> {
        let start = self.tok().span;
        self.expect_kw(Kw::Temp)?;
        self.expect_kw(Kw::Relation)?;
        let name = self.lower("a relation name")?;
        self.expect(Tok::Colon, "`:`")?;
        let ty = self.app_type()?;
        let key = if self.eat_kw(Kw::Key) {
            let fields = self.key_fields()?;
            let generate = self.generated()?;
            Some((fields, generate))
        } else {
            None
        };
        Ok(TempDecl {
            name,
            ty,
            key,
            span: self.span_from(start),
        })
    }

    fn role_stmt(&mut self) -> PResult<Option<RoleStmt>> {
        let verb = match self.tok().tok {
            Tok::Lower if self.peek_word(1, "role") => self.text_of(self.tok()),
            _ => return Ok(None),
        };
        let stmt = match verb {
            "create" => {
                self.bump();
                self.bump();
                RoleStmt::Create(self.lower("a role name")?)
            }
            "drop" => {
                self.bump();
                self.bump();
                RoleStmt::Drop(self.lower("a role name")?)
            }
            "alter" => {
                self.bump();
                self.bump();
                let role = self.lower("a role name")?;
                self.expect_word("password")?;
                RoleStmt::AlterPassword(role, self.prefix_e()?)
            }
            _ => return Ok(None),
        };
        Ok(Some(stmt))
    }

    fn peer_stmt(&mut self) -> PResult<Option<PeerStmt>> {
        if !self.peek_word(1, "peer") {
            return Ok(None);
        }
        if self.eat_word("map") {
            self.bump();
            let pattern = self.text_lit()?;
            self.expect_word("on")?;
            let listener = self.lower("a listener name")?;
            self.expect_word("to")?;
            let role = self.lower("a role name")?;
            return Ok(Some(PeerStmt::Map {
                pattern,
                listener,
                role,
            }));
        }
        if self.eat_word("unmap") {
            self.bump();
            let ordinal = self.int_lit()?;
            self.expect_word("on")?;
            let listener = self.lower("a listener name")?;
            return Ok(Some(PeerStmt::Unmap { ordinal, listener }));
        }
        Ok(None)
    }

    /// A command, recognised by its leading words; `None` when the text is not one.
    fn command(&mut self) -> PResult<Option<Command>> {
        if self.tok().tok != Tok::Lower {
            return Ok(None);
        }
        let word = self.text_of(self.tok());
        let next_is = |p: &Self, w: &str| p.peek_word(1, w);
        let next_lower = self.peek(1).tok == Tok::Lower;
        let cmd = match word {
            "explain" => {
                self.bump();
                let analyze = self.eat_word("analyze");
                Command::Explain {
                    analyze,
                    target: self.explain_target()?,
                }
            }
            "pin" if next_is(self, "plan") => {
                self.bump();
                self.bump();
                Command::PinPlan(self.prefix_e()?)
            }
            "unpin" if next_is(self, "plan") => {
                self.bump();
                self.bump();
                Command::UnpinPlan(self.prefix_e()?)
            }
            "cancel" if next_is(self, "call") => {
                self.bump();
                self.bump();
                Command::CancelCall(self.prefix_e()?)
            }
            "terminate" if next_is(self, "session") => {
                self.bump();
                self.bump();
                Command::TerminateSession(self.prefix_e()?)
            }
            "vacuum" if next_lower => {
                self.bump();
                Command::Vacuum(self.qlower("a relation name")?)
            }
            "verify" => {
                self.bump();
                let target = if self.at(Tok::Lower) && !self.at_item_start() {
                    Some(self.qlower("a relation name")?)
                } else {
                    None
                };
                Command::Verify(target)
            }
            "reindex" if next_lower => {
                self.bump();
                let relation = self.qlower("a relation name")?;
                let index = if self.eat_word("index") {
                    let fields = self.index_fields()?;
                    self.expect_kw(Kw::In)?;
                    Some((fields, self.ctor_scope()?))
                } else {
                    None
                };
                let online = if self.eat_word("online") {
                    Some(true)
                } else if self.eat_word("offline") {
                    Some(false)
                } else {
                    None
                };
                Command::Reindex {
                    relation,
                    index,
                    online,
                }
            }
            "compact" => {
                self.bump();
                Command::Compact
            }
            "finalize" if next_is(self, "upgrade") => {
                self.bump();
                self.bump();
                Command::FinalizeUpgrade
            }
            "retire" if self.peek(1).tok == Tok::Kw(Kw::Bundle) => {
                self.bump();
                self.bump();
                Command::RetireBundle(self.bundle_name()?)
            }
            "supersede" if self.peek(1).tok == Tok::Kw(Kw::Bundle) => {
                self.bump();
                self.bump();
                let old = self.bundle_name()?;
                self.expect_kw(Kw::By)?;
                Command::SupersedeBundle(old, self.bundle_name()?)
            }
            "export" => {
                self.bump();
                let source = self.or_e()?;
                self.expect_word("to")?;
                let path = self.text_lit()?;
                let sums_text = if self.eat_word("sums") {
                    self.expect_word("text")?;
                    true
                } else {
                    false
                };
                Command::Export {
                    source,
                    path,
                    sums_text,
                }
            }
            "register" if self.peek(1).tok == Tok::Kw(Kw::Bundle) => {
                self.bump();
                self.bump();
                Command::RegisterBundle(self.text_lit()?)
            }
            "apply" if next_is(self, "migrations") => {
                self.bump();
                self.bump();
                let mut paths = vec![self.text_lit()?];
                while self.at(Tok::Text) {
                    paths.push(self.text_lit()?);
                }
                Command::ApplyMigrations(paths)
            }
            "print" if self.peek(1).tok == Tok::Kw(Kw::Schema) => {
                self.bump();
                self.bump();
                Command::PrintSchema
            }
            "archive" if next_is(self, "audit") => {
                self.bump();
                self.bump();
                self.expect_word("through")?;
                let through = self.add_e()?;
                self.expect_word("to")?;
                Command::ArchiveAudit {
                    through,
                    path: self.text_lit()?,
                }
            }
            "rotate" if self.peek(1).tok == Tok::Kw(Kw::Key) => {
                self.bump();
                self.bump();
                if self.eat_word("kek") {
                    Command::RotateKek(self.text_lit()?)
                } else {
                    self.expect_word("dek")?;
                    Command::RotateDek
                }
            }
            "destroy" if self.peek(1).tok == Tok::Kw(Kw::Key) => {
                self.bump();
                self.bump();
                self.expect_word("dek")?;
                Command::DestroyDek(self.int_lit()?)
            }
            "dump" if next_is(self, "to") => {
                self.bump();
                self.bump();
                let path = self.text_lit()?;
                let encrypt = if self.eat_word("encrypt") {
                    Some(self.prefix_e()?)
                } else {
                    None
                };
                Command::Dump { path, encrypt }
            }
            "restore" if self.peek(1).tok == Tok::Kw(Kw::From) => {
                self.bump();
                self.bump();
                let from = self.text_lit()?;
                self.expect_word("to")?;
                let to = self.text_lit()?;
                let point = if self.at_word("before") {
                    self.bump();
                    self.expect_word("version")?;
                    Some(RestorePoint::BeforeVersion(self.int_lit()?))
                } else if self.at_word("at") && self.peek_word(1, "instant") {
                    self.bump();
                    self.bump();
                    Some(RestorePoint::AtInstant(self.add_e()?))
                } else if self.at_word("at") && self.peek_word(1, "position") {
                    self.bump();
                    self.bump();
                    Some(RestorePoint::AtPosition(self.add_e()?))
                } else {
                    None
                };
                Command::Restore { from, to, point }
            }
            "backup" if next_is(self, "to") => {
                self.bump();
                self.bump();
                let path = self.text_lit()?;
                let incremental = self.eat_word("incremental");
                Command::Backup { path, incremental }
            }
            _ => return Ok(None),
        };
        Ok(Some(cmd))
    }

    fn explain_target(&mut self) -> PResult<ExplainTarget> {
        if self.at_bundle_name() {
            let bundle = self.bundle_name()?;
            self.expect(Tok::ColonColon, "`::`")?;
            let query = self.lower("a query name")?;
            let args = self.call_args()?;
            return Ok(ExplainTarget::Registered {
                bundle,
                query,
                args,
            });
        }
        if self.at_kw(Kw::Transact) {
            return Ok(ExplainTarget::Transact(self.transact_block()?));
        }
        if matches!(
            self.tok().tok,
            Tok::Kw(Kw::Insert | Kw::Update | Kw::Delete | Kw::Fail | Kw::Skip)
        ) {
            return Ok(ExplainTarget::Write(self.write_expr()?));
        }
        Ok(ExplainTarget::Expr(self.expr()?))
    }
}
