//! The gate a role's call passes: the judgements of its principal, capabilities and grants, at
//! submit and again when it leaves the queue, each refusal for privilege recorded in the audit.

use std::sync::Arc;

use adtb_lang::ast::{self, AdhocItem};
use adtb_lang::check::Command;
use adtb_lang::permission::{self, Requirements};
use adtb_lang::schema::{ModuleDef, Schema, ValueDef};
use adtb_lang::types::{RelId, Ty};

use crate::exec::{Priority, Val};
use crate::runtime::Runtime;

use super::super::admitted::{Admitted, Authority, CALL_BYTES, Queued};
use super::super::bundle::bundle_name;
use super::super::cause::past_limits;
use super::super::command::{Need, free_params, need};
use super::super::dump::read_by_dump;
use super::super::registered::{Callee, Registered};
use super::super::text::{Issuer, Prepared, TextError, declared};
use super::super::{Engine, Error, OWNER_ROLE, RejectReason};
use super::adhoc::{Holds, refused_text, schema_relations, unanalysed};
use super::binding::{RoleBinding, Target, holds_call, owns};
use super::call::{RoleCall, Work};
use super::grant::Calls;
use super::session::{RoleSession, text_hash};
use super::tables::{CallOrigin, CallRow, Tables, service};
use super::task::{TaskOptions, args_hash};
use super::{Capability, RoleAuthority, denied};

type Opened<'e> = std::sync::RwLockReadGuard<'e, bool>;

/// The authority of a role's call, judged by this gate alone: the issuer of its text, which
/// holds what the role held when judged.
pub(in crate::api) struct Judged(Option<Box<Issuer>>);

impl Judged {
    pub(in crate::api) fn issuer(self) -> Option<Issuer> {
        self.0.map(|i| *i)
    }
}

impl<R: Runtime> RoleSession<R> {
    /// Admit ad hoc `text` of the session, each free name typed and bound by `params`, to run as
    /// a task under `opts`. Returns the admitted program, its call and the engine held open.
    pub(super) fn admit_text<'e>(
        &self,
        e: &'e Arc<Engine<R>>,
        text: &str,
        params: &[(&str, Ty, Val)],
        opts: &TaskOptions,
    ) -> Result<(Admitted, RoleCall<R>, Opened<'e>), Error> {
        let authority = RoleAuthority {
            engine: self.engine.clone(),
        };
        let assertion = opts.call.assert.clone();
        let hash = text_hash(text);
        let origin = CallOrigin::AdHoc(hash);
        let caller = authority.admit_caller(self, &hash, assertion.as_ref())?;
        let names: Vec<String> = params.iter().map(|(n, ..)| (*n).to_owned()).collect();
        let parsed = Prepared::parsed(text);
        let mut issuer = match &parsed {
            Ok(items) => authority.admit_items(self, e, items, &names, &caller, &hash)?,
            Err(_) => None,
        };
        let _open = e.enter()?;
        let held = (self.name != OWNER_ROLE)
            .then(|| authority.holds(self))
            .transpose()?;
        let unseen = held.as_ref().filter(|h| !h.schema);
        let refused = || {
            if let Some(quarantined) = e.refusal() {
                return quarantined;
            }
            match e.audit_denied(&self.name, &caller, &origin) {
                Ok(()) => refused_text(&self.name),
                Err(err) => err,
            }
        };
        let failed = |args: &[u8; 32], err: Error| {
            e.audit_call(
                &CallRow {
                    role: &self.name,
                    caller: &caller,
                    origin: &origin,
                    args,
                    failed: Some(&err),
                },
                None,
            )
            .map_or_else(|audit| audit, |()| err)
        };
        if unseen.is_some_and(|h| {
            params
                .iter()
                .any(|(_, ty, _)| !declared(e.schema(), ty) || !h.names.may_type(e.schema(), ty))
        }) {
            return Err(refused());
        }
        if let Some((name, ..)) = params.iter().find(|(_, ty, _)| !declared(e.schema(), ty)) {
            let err = Error::invalid(format!("parameter `{name}` has no type of the schema"));
            return Err(failed(&names_hash(&names), err));
        }
        let args = match parsed.as_ref().ok().and_then(|items| passphrases(items)) {
            Some(secret) => {
                let typed: Vec<_> = params
                    .iter()
                    .filter(|(n, ..)| !secret.contains(n))
                    .map(|(n, t, v)| (*n, t, v))
                    .collect();
                match args_hash(e.schema(), &typed) {
                    Ok(args) => args,
                    Err(err) => return Err(failed(&names_hash(&names), err)),
                }
            }
            None => names_hash(&names),
        };
        let capability_only = parsed
            .as_ref()
            .is_ok_and(|items| items.iter().all(is_capability_command));
        let hides = unseen.is_some() && parsed.is_ok() && !capability_only;
        let scope = unseen.map(|h| {
            let commanded = match &parsed {
                Ok(items) if capability_only => commanded(e.schema(), items),
                _ => Vec::new(),
            };
            h.names.naming(commanded).scope(e.schema())
        });
        let checked = parsed
            .map_err(|err| TextError::Other(e.refusal().unwrap_or(err)))
            .and_then(|items| {
                let p = Prepared::naming(
                    &e.compiled,
                    scope.as_ref(),
                    &e.explained(),
                    &items,
                    params,
                    caller.clone(),
                )?;
                let p = e.admit_text(Ok(p)).map_err(TextError::Other)?;
                Ok((p, items))
            });
        let (p, items) = match checked {
            Ok(checked) => checked,
            Err(TextError::Checker(_)) if hides => return Err(refused()),
            Err(TextError::Checker(err)) => {
                let err = e.refusal().unwrap_or(err);
                return Err(failed(&args, err));
            }
            Err(TextError::Other(err)) => return Err(failed(&args, err)),
        };
        if let Some(issuer) = &mut issuer {
            issuer.holds = Some(authority.admit_prepared(self, e, &p, &caller, issuer, &args)?);
        }
        let readmission = (self.name != OWNER_ROLE).then(|| Readmission::Text {
            session: self.clone(),
            text: hash,
            items,
            params: names,
            args,
            assertion,
        });
        let call = RoleCall {
            role: self.name.clone(),
            session: self.serial,
            origin,
            args,
            readmission,
        };
        let queued = Queued::new(
            Priority::AdHoc,
            opts.call.priority,
            opts.call.deadline,
            CALL_BYTES + text.len() + p.bytes(),
        );
        let admitted =
            e.admit_program(p, &queued, Authority::Role(Judged(issuer.map(Box::new))))?;
        Ok((admitted, call, _open))
    }
}

impl<R: Runtime> RoleBinding<R> {
    /// Admit the call of `ordinal` with `args` to run as a task under `opts`. Returns the
    /// admitted program, the work of its result, its call and the engine held open.
    pub(super) fn admit_registered<'e>(
        &self,
        e: &'e Arc<Engine<R>>,
        ordinal: u32,
        args: &[Val],
        opts: &TaskOptions,
    ) -> Result<(Admitted, Work, RoleCall<R>, Opened<'e>), Error> {
        let session = self.session();
        let callee = self
            .registered()
            .callee(ordinal)
            .ok_or_else(|| Error::rejected(RejectReason::UnknownQuery))?;
        let origin = self.origin(&callee, ordinal);
        let assertion = opts.call.assert.clone();
        let caller = session.authority().admit_call(
            session,
            self.registered(),
            &callee,
            &origin,
            assertion.as_ref(),
        )?;
        let open = e.enter()?;
        self.registered().arguments(ordinal, args)?;
        let typed: Vec<_> = callee
            .names
            .iter()
            .zip(callee.types.iter().zip(args))
            .map(|(n, (t, v))| (n.as_str(), t, v))
            .collect();
        let hash = args_hash(self.registered().schema(), &typed)?;
        let (bound, p) = match self.registered().bound(e, ordinal, args, caller.clone()) {
            Ok(bound) => bound,
            Err(err) => {
                e.audit_call(
                    &CallRow {
                        role: &session.name,
                        caller: &caller,
                        origin: &origin,
                        args: &hash,
                        failed: Some(&err),
                    },
                    None,
                )?;
                return Err(err);
            }
        };
        let call = RoleCall {
            role: session.name.clone(),
            session: session.serial,
            origin,
            args: hash,
            readmission: Some(Readmission::Registered {
                binding: self.clone(),
                ordinal,
                assertion,
            }),
        };
        let queued = self
            .registered()
            .queued(args, opts.call.priority, opts.call.deadline);
        let admitted = e.admit_program(p, &queued, Authority::Role(Judged(None)))?;
        Ok((admitted, Work::Registered(Box::new(bound)), call, open))
    }

    /// Refuse the call of `ordinal`, leaving the queue, as [`submit`](RoleBinding::submit) does.
    fn recheck(&self, e: &Engine<R>, ordinal: u32, assertion: Option<&Val>) -> Result<(), Error> {
        let callee = self
            .registered()
            .callee(ordinal)
            .expect("a submitted call's ordinal names a callable");
        let origin = self.origin(&callee, ordinal);
        self.session().authority().admit_call(
            self.session(),
            self.registered(),
            &callee,
            &origin,
            assertion,
        )?;
        e.admit(self.registered().name(), callee.name).map(drop)
    }
}

impl<R: Runtime> RoleAuthority<R> {
    /// Returns the principal ad hoc text of hash `text` of `session` runs for, refusing a dropped
    /// role and a role lacking `adhoc`, or `assert` when `assertion` is given, with `Denied`
    /// recorded in the audit. A database quarantined whole has no grants to read, and the embedded
    /// owner holds `adhoc` without them.
    fn admit_caller(
        &self,
        session: &RoleSession<R>,
        text: &[u8; 32],
        assertion: Option<&Val>,
    ) -> Result<Val, Error> {
        let e = self.engine.upgrade().ok_or_else(Error::closed)?;
        let service = service(e.schema(), &session.name);
        let origin = CallOrigin::AdHoc(*text);
        if session.name == OWNER_ROLE && assertion.is_none() && e.db.admit_database().is_err() {
            return Ok(service);
        }
        self.demand(session, &service, &origin, Capability::Adhoc)?;
        if assertion.is_some() {
            self.demand(session, &service, &origin, Capability::Assert)?;
        }
        self.admit_principal(session, &service, &origin, assertion)
    }

    /// Returns the principal of `session`, recording a `Denied` refusal for `service`.
    pub(super) fn admit_principal(
        &self,
        session: &RoleSession<R>,
        service: &Val,
        origin: &CallOrigin,
        assertion: Option<&Val>,
    ) -> Result<Val, Error> {
        let got = self.principal(session, assertion);
        self.record_refusal(session, service, origin, got)
    }

    /// Refuse parsed ad hoc `items` of `session` unless the role may issue the commands among
    /// them and they hold no statement of management, with `Denied` recorded in the audit, before
    /// the checker reads them. Returns the issuer of a role but the schema owner;
    /// [`RoleAuthority::admit_prepared`] decides its queries and writes once they are checked.
    fn admit_items(
        &self,
        session: &RoleSession<R>,
        e: &Engine<R>,
        items: &[AdhocItem],
        params: &[String],
        caller: &Val,
        text: &[u8; 32],
    ) -> Result<Option<Issuer>, Error> {
        if session.name == OWNER_ROLE {
            return Ok(None);
        }
        let params = free_params(e.schema(), params);
        let origin = CallOrigin::AdHoc(*text);
        let refuse = |detail: String| -> Result<Option<Issuer>, Error> {
            if let Some(quarantined) = e.refusal() {
                return Err(quarantined);
            }
            e.audit_denied(&session.name, caller, &origin)?;
            Err(denied(detail))
        };
        for item in items {
            let c = match item {
                AdhocItem::Command(c, _) => c,
                AdhocItem::Expr(_)
                | AdhocItem::Write(_)
                | AdhocItem::Transact(_)
                | AdhocItem::Temp(_) => continue,
                AdhocItem::Grant(_)
                | AdhocItem::Revoke(_)
                | AdhocItem::Role(..)
                | AdhocItem::Peer(..) => {
                    return refuse(
                        "management statements run only for the embedded owner's role".to_owned(),
                    );
                }
            };
            match need(c, &params) {
                Need::Capabilities(capabilities) => {
                    for capability in capabilities {
                        self.demand(session, caller, &origin, *capability)?;
                    }
                }
                Need::BundleOwner(bundle) => {
                    let name = bundle_name(bundle);
                    let deployed = e.deployed.read(&e.rt);
                    let owner = deployed.bundles.get(&name).map(|r| deployed.owner_of(r));
                    drop(deployed);
                    if owner != Some(session.id.0) {
                        return refuse(format!(
                            "role `{}` does not own bundle {name}",
                            session.name
                        ));
                    }
                }
                Need::SchemaOwner => {
                    return refuse(
                        "this command as written runs only for the embedded owner's role"
                            .to_owned(),
                    );
                }
            }
        }
        Ok(Some(Issuer {
            role: session.name.clone(),
            text: *text,
            session: session.serial,
            holds: None,
        }))
    }

    fn demand(
        &self,
        session: &RoleSession<R>,
        caller: &Val,
        origin: &CallOrigin,
        capability: Capability,
    ) -> Result<(), Error> {
        let got = self.require(session, capability, false);
        self.record_refusal(session, caller, origin, got)
    }

    /// Refuse text `p` of `session` unless the role now holds all it reads and writes, with
    /// `Denied` recorded in the audit, and name what it lacks only to a role that may read the
    /// schema. Returns what the role holds.
    fn admit_prepared(
        &self,
        session: &RoleSession<R>,
        e: &Engine<R>,
        p: &Prepared,
        caller: &Val,
        issuer: &Issuer,
        args: &[u8; 32],
    ) -> Result<Holds, Error> {
        let holds = self.holds(session)?;
        let params: Vec<_> = p.params().iter().map(|(v, _)| *v).collect();
        let normal = e.db.normal();
        let origin = CallOrigin::AdHoc(issuer.text);
        let needs = |root| {
            permission::normal(p.schema(), normal, p.terms(), root, &params).map_err(|err| {
                if !holds.schema && past_limits(&err) {
                    if let Some(quarantined) = e.refusal() {
                        return quarantined;
                    }
                    return e
                        .audit_denied(&session.name, caller, &origin)
                        .map_or_else(|audit| audit, |()| refused_text(&session.name));
                }
                let err = unanalysed(err);
                e.audit_call(
                    &CallRow {
                        role: &session.name,
                        caller,
                        origin: &origin,
                        args,
                        failed: Some(&err),
                    },
                    None,
                )
                .map_or_else(|audit| audit, |()| err)
            })
        };
        let mut lacking = Vec::new();
        let roots = p
            .items()
            .iter()
            .copied()
            .chain(p.commands().iter().flat_map(|c| c.command.terms()));
        for root in roots {
            lacking.extend(holds.lacking(p.schema(), &needs(root)?));
        }
        if p.commands()
            .iter()
            .any(|c| matches!(c.command, Command::Dump { .. }))
        {
            let mut read = read_by_dump(p.schema());
            read.extend(schema_relations(p.schema()));
            let dumped = Requirements::whole(p.schema(), read);
            lacking.extend(holds.lacking(p.schema(), &dumped));
        }
        let uncalled = p.functions().iter().find(|f| {
            let def = &p.schema().functions[f.0 as usize];
            !p.schema().module(def.module).engine && !holds.may_call(**f)
        });
        let unseen = p
            .commands()
            .iter()
            .any(|c| matches!(c.command, Command::PrintSchema))
            && !holds.schema;
        let Some(detail) = lacking
            .first()
            .map(|r| r.text(p.schema()))
            .or(uncalled.map(|f| {
                let name = &p.schema().functions[f.0 as usize].name;
                (format!("calls `{name}`"), "call")
            }))
            .or(unseen.then(|| ("reads the schema".to_owned(), "read")))
        else {
            return Ok(holds);
        };
        if let Some(quarantined) = e.refusal() {
            return Err(quarantined);
        }
        e.audit_denied(&session.name, caller, &origin)?;
        Err(if holds.schema {
            denied(format!(
                "role `{}` {} and lacks {}",
                session.name, detail.0, detail.1
            ))
        } else {
            refused_text(&session.name)
        })
    }
    /// Returns the principal of a call of `callee` of `registered`, from one observation of the
    /// role and its grants. Refuse with one `Denied` detail, recorded in the audit, a stale
    /// session, an assertion without `assert`, and a role holding no grant of the call.
    fn admit_call(
        &self,
        session: &RoleSession<R>,
        registered: &Registered<R>,
        callee: &Callee<'_>,
        origin: &CallOrigin,
        assertion: Option<&Val>,
    ) -> Result<Val, Error> {
        let bundle = registered.name();
        let observed = self.access(false, |e, tx, id, _| {
            let t = Tables::new(e.schema(), id);
            let principal = self.derive(session, &t, e, tx, assertion)?;
            let stored = e.deployed.read(&e.rt);
            if owns(&stored, stored.bundles.get(bundle), session.id.0) {
                return Ok((principal, true));
            }
            let target = Target::new(bundle, callee.name, callee.function);
            let grants = t.grants(e, tx)?;
            let held = holds_call(e, &Calls::new(e), &grants, &session.name, &target);
            Ok((principal, held))
        });
        let caller = match observed {
            Ok((principal, true)) => return Ok(principal),
            Ok((principal, false)) => principal,
            Err(Error::Rejected {
                reason: RejectReason::Denied { .. },
            }) => {
                let e = self.engine.upgrade().ok_or_else(Error::closed)?;
                service(e.schema(), &session.name)
            }
            Err(err) => return Err(err),
        };
        let refused = denied(format!("role `{}` may not make this call", session.name));
        self.record_refusal(session, &caller, origin, Err(refused))
    }
}

/// What a task rechecks when it leaves the queue.
pub(super) enum Readmission<R: Runtime> {
    Text {
        session: RoleSession<R>,
        text: [u8; 32],
        items: Vec<AdhocItem>,
        args: [u8; 32],
        params: Vec<String>,
        assertion: Option<Val>,
    },
    Registered {
        binding: RoleBinding<R>,
        ordinal: u32,
        assertion: Option<Val>,
    },
}

impl<R: Runtime> Readmission<R> {
    /// Refuse the call with `Denied`, recorded in the audit, unless the session's role may still
    /// make it, and a registered call whose bundle has left its lifecycle. Returns what the role
    /// of checked text `admitted` holds now.
    pub(super) fn check(&self, e: &Engine<R>, admitted: &Admitted) -> Result<Option<Holds>, Error> {
        match self {
            Readmission::Text {
                session,
                text,
                items,
                params,
                args,
                assertion,
            } => {
                let authority = session.authority();
                let caller = authority.admit_caller(session, text, assertion.as_ref())?;
                let issuer = authority.admit_items(session, e, items, params, &caller, text)?;
                issuer
                    .map(|issuer| {
                        let p = admitted.prepared();
                        authority.admit_prepared(session, e, p, &caller, &issuer, args)
                    })
                    .transpose()
            }
            Readmission::Registered {
                binding,
                ordinal,
                assertion,
            } => binding
                .recheck(e, *ordinal, assertion.as_ref())
                .map(|()| None),
        }
    }
}

/// Returns whether `item` is a command a capability alone decides, holding no expression the
/// role's grants decide.
fn is_capability_command(item: &AdhocItem) -> bool {
    let AdhocItem::Command(command, _) = item else {
        return false;
    };
    match command {
        ast::Command::Vacuum(_)
        | ast::Command::Verify(_)
        | ast::Command::Reindex { .. }
        | ast::Command::Compact
        | ast::Command::FinalizeUpgrade
        | ast::Command::RetireBundle(_)
        | ast::Command::SupersedeBundle(..)
        | ast::Command::RegisterBundle(_)
        | ast::Command::ApplyMigrations(_)
        | ast::Command::PrintSchema
        | ast::Command::RotateKek(_)
        | ast::Command::RotateDek
        | ast::Command::DestroyDek(_)
        | ast::Command::Backup { .. } => true,
        ast::Command::Explain { .. }
        | ast::Command::PinPlan(_)
        | ast::Command::UnpinPlan(_)
        | ast::Command::CancelCall(_)
        | ast::Command::TerminateSession(_)
        | ast::Command::SetSetting(..)
        | ast::Command::Export { .. }
        | ast::Command::ArchiveAudit { .. }
        | ast::Command::Dump { .. }
        | ast::Command::Restore { .. } => false,
    }
}

/// Returns the parameters `items` give as the passphrase of a `dump` or `restore`, or `None` when
/// an operand is not a bare name.
fn passphrases(items: &[AdhocItem]) -> Option<Vec<&str>> {
    let operands = items.iter().filter_map(|item| match item {
        AdhocItem::Command(ast::Command::Dump { encrypt, .. }, _) => encrypt.as_ref(),
        AdhocItem::Command(ast::Command::Restore { decrypt, .. }, _) => decrypt.as_deref(),
        _ => None,
    });
    let names = operands.map(|k| match &k.kind {
        ast::ExprKind::Name(q) if q.module.is_none() => Some(q.name.text.as_str()),
        _ => None,
    });
    names.collect()
}

/// Returns every relation of a user module the relation names in `items` may denote.
fn commanded(schema: &Schema, items: &[AdhocItem]) -> Vec<RelId> {
    let mut out = Vec::new();
    for item in items {
        let AdhocItem::Command(
            ast::Command::Vacuum(name)
            | ast::Command::Verify(Some(name))
            | ast::Command::Reindex { relation: name, .. },
            _,
        ) = item
        else {
            continue;
        };
        let denoted = |module: &ModuleDef| match module.values.get(&name.name.text) {
            Some(ValueDef::Relation(r)) if !module.engine => Some(*r),
            _ => None,
        };
        match &name.module {
            Some(m) => out.extend(
                schema
                    .module_named(&m.text)
                    .and_then(|id| denoted(schema.module(id))),
            ),
            None => out.extend(schema.modules.iter().filter_map(denoted)),
        }
    }
    out
}

/// Returns the audit's arguments hash of a text whose values do not print: that of their names.
fn names_hash(names: &[String]) -> [u8; 32] {
    let mut sorted: Vec<&String> = names.iter().collect();
    sorted.sort();
    let mut hasher = blake3::Hasher::new();
    for name in sorted {
        hasher.update(name.as_bytes());
        hasher.update(&[0]);
    }
    *hasher.finalize().as_bytes()
}
