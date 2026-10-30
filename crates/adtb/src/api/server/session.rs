use std::sync::Weak;

use adtb_lang::ast::AdhocItem;

use crate::exec::Val;
use crate::runtime::Runtime;

use super::super::bundle::bundle_name;
use super::super::command::{Need, need};
use super::super::text::Issuer;
use super::super::{Engine, Error, OWNER_ROLE, RejectReason};
use super::store;
use super::tables::{RoleId, Tables};
use super::{Capability, RoleAuthority, denied, require};

#[derive(Clone, Debug)]
pub struct RoleSession<R: Runtime = adtb_sys::os::OsRuntime> {
    pub(super) engine: Weak<Engine<R>>,
    pub(super) id: RoleId,
    pub(super) name: String,
}

impl<R: Runtime> RoleSession<R> {
    /// Returns the role name authenticated when the session was issued.
    pub fn role_name(&self) -> &str {
        &self.name
    }
}

impl<R: Runtime> RoleAuthority<R> {
    /// Derive a principal after checking the session and assertion capability.
    pub fn principal(
        &self,
        session: &RoleSession<R>,
        assertion: Option<&Val>,
    ) -> Result<Val, Error> {
        self.access(false, |e, tx, id, _| {
            let t = Tables::new(&e.schema, id);
            self.checked(session, &t, e, tx)?;
            let Some(assertion) = assertion else {
                return Ok(service(e, session));
            };
            require(&t.grants(e, tx)?, session, Capability::Assert, false)?;
            e.principal(Some(assertion))
        })
    }

    /// Returns the principal ad hoc `text` of `session` runs for, refusing a dropped role and a
    /// role lacking `adhoc`, or `assert` when `assertion` is given, with `Denied` recorded in the
    /// audit. A database quarantined whole has no grants to read, and the embedded owner holds
    /// `adhoc` without them.
    pub(super) fn admit_caller(
        &self,
        session: &RoleSession<R>,
        text: &str,
        assertion: Option<&Val>,
    ) -> Result<Val, Error> {
        let e = self.engine.upgrade().ok_or_else(Error::closed)?;
        let service = service(&e, session);
        if session.name == OWNER_ROLE && assertion.is_none() && e.db.admit_database().is_err() {
            return Ok(service);
        }
        let hash = text_hash(text);
        self.demand(session, &service, &hash, Capability::Adhoc)?;
        if assertion.is_some() {
            self.demand(session, &service, &hash, Capability::Assert)?;
        }
        self.principal(session, assertion)
    }

    /// Refuse parsed ad hoc `items` of `session` unless the role may issue all of them, with
    /// `Denied` recorded in the audit, before anything checks them against the schema. Returns
    /// the issuer of a role but the schema owner, which issues only commands that capabilities or
    /// bundle ownership decide.
    pub(super) fn admit_items(
        &self,
        session: &RoleSession<R>,
        e: &Engine<R>,
        items: &[AdhocItem],
        caller: &Val,
        text: &str,
    ) -> Result<Option<Issuer>, Error> {
        if session.name == OWNER_ROLE {
            return Ok(None);
        }
        let hash = text_hash(text);
        let refuse = |detail: String| -> Result<Option<Issuer>, Error> {
            e.audit_denied(&session.name, caller, &hash)?;
            Err(denied(detail))
        };
        let commands: Vec<_> = items
            .iter()
            .filter_map(|item| match item {
                AdhocItem::Command(c, _) => Some(c),
                _ => None,
            })
            .collect();
        if commands.len() != items.len() {
            return refuse("ad hoc statements run only for the embedded owner's role".to_owned());
        }
        for c in commands {
            match need(c) {
                Need::Capabilities(capabilities) => {
                    for capability in capabilities {
                        self.demand(session, caller, &hash, *capability)?;
                    }
                }
                Need::BundleOwner(bundle) => {
                    let name = bundle_name(bundle);
                    let deployed = e.deployed.lock(&e.rt);
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
            text: hash,
        }))
    }

    fn demand(
        &self,
        session: &RoleSession<R>,
        caller: &Val,
        text: &[u8; 32],
        capability: Capability,
    ) -> Result<(), Error> {
        let got = self.require(session, capability, false);
        if matches!(
            got,
            Err(Error::Rejected {
                reason: RejectReason::Denied { .. }
            })
        ) {
            let e = self.engine.upgrade().ok_or_else(Error::closed)?;
            e.audit_denied(&session.name, caller, text)?;
        }
        got
    }
}

impl<R: Runtime> Engine<R> {
    /// Record, in a transaction of its own, that `role` running for `caller` was refused ad hoc
    /// text of hash `text`.
    pub(in super::super) fn audit_denied(
        &self,
        role: &str,
        caller: &Val,
        text: &[u8; 32],
    ) -> Result<(), Error> {
        let _open = self.enter()?;
        store::run(self, true, &mut |e, tx, id, changes| {
            Tables::new(&e.schema, id).audit_denied(e, tx, changes, role, caller, text)
        })
    }
}

fn service<R: Runtime>(e: &Engine<R>, session: &RoleSession<R>) -> Val {
    let def = e.schema.ty(e.schema.principal);
    let service = def
        .ctor_index("Service")
        .expect("the Principal type declares Service");
    Val::ctor(service, vec![Val::text(&session.name)])
}

fn text_hash(text: &str) -> [u8; 32] {
    *blake3::hash(text.as_bytes()).as_bytes()
}
