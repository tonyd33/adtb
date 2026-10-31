use std::sync::Weak;
use std::sync::atomic::Ordering;

use adtb_lang::ast::AdhocItem;
use adtb_lang::types::RelId;

use crate::exec::Val;
use crate::runtime::Runtime;
use crate::write::Txn;

use super::super::bundle::bundle_name;
use super::super::command::{Need, free_params, need};
use super::super::text::Issuer;
use super::super::{Engine, Error, OWNER_ROLE, RejectReason, TxnId};
use super::binding::RoleBinding;
use super::store::{self, Changes};
use super::tables::{CallOrigin, CallRow, RoleId, Tables, service};
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

    pub(super) fn authority(&self) -> RoleAuthority<R> {
        RoleAuthority {
            engine: self.engine.clone(),
        }
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
            self.derive(session, &Tables::new(&e.schema, id), e, tx, assertion)
        })
    }

    pub(super) fn derive(
        &self,
        session: &RoleSession<R>,
        t: &Tables<'_>,
        e: &Engine<R>,
        tx: &Txn,
        assertion: Option<&Val>,
    ) -> Result<Val, Error> {
        self.checked(session, t, e, tx)?;
        let Some(assertion) = assertion else {
            return Ok(service(&e.schema, &session.name));
        };
        require(&t.grants(e, tx)?, session, Capability::Assert, false)?;
        e.principal(Some(assertion))
    }

    /// Returns the principal ad hoc text of hash `text` of `session` runs for, refusing a dropped
    /// role and a role lacking `adhoc`, or `assert` when `assertion` is given, with `Denied`
    /// recorded in the audit. A database quarantined whole has no grants to read, and the embedded
    /// owner holds `adhoc` without them.
    pub(super) fn admit_caller(
        &self,
        session: &RoleSession<R>,
        text: &[u8; 32],
        assertion: Option<&Val>,
    ) -> Result<Val, Error> {
        let e = self.engine.upgrade().ok_or_else(Error::closed)?;
        let service = service(&e.schema, &session.name);
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

    /// Refuse parsed ad hoc `items` of `session` unless the role may issue all of them, with
    /// `Denied` recorded in the audit, before anything checks them against the schema. Returns
    /// the issuer of a role but the schema owner, which issues only commands that capabilities or
    /// bundle ownership decide.
    pub(super) fn admit_items(
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
        let params = free_params(&e.schema, params);
        let origin = CallOrigin::AdHoc(*text);
        let refuse = |detail: String| -> Result<Option<Issuer>, Error> {
            if let Some(quarantined) = e.refusal() {
                return Err(quarantined);
            }
            e.audit_denied(&session.name, caller, &origin)?;
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
            match need(c, &params) {
                Need::Capabilities(capabilities) => {
                    for capability in capabilities {
                        self.demand(session, caller, &origin, *capability)?;
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
            text: *text,
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

    pub(super) fn record_refusal<T>(
        &self,
        session: &RoleSession<R>,
        caller: &Val,
        origin: &CallOrigin,
        got: Result<T, Error>,
    ) -> Result<T, Error> {
        if matches!(
            got,
            Err(Error::Rejected {
                reason: RejectReason::Denied { .. }
            })
        ) {
            let e = self.engine.upgrade().ok_or_else(Error::closed)?;
            if let Some(quarantined) = e.refusal() {
                return Err(quarantined);
            }
            e.audit_denied(&session.name, caller, origin)?;
        }
        got
    }
}

/// What a task rechecks when it leaves the queue.
pub(super) enum Readmission<R: Runtime> {
    Text {
        session: RoleSession<R>,
        text: [u8; 32],
        items: Vec<AdhocItem>,
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
    /// make it, and a registered call whose bundle has left its lifecycle.
    pub(super) fn check(&self, e: &Engine<R>) -> Result<(), Error> {
        match self {
            Readmission::Text {
                session,
                text,
                items,
                params,
                assertion,
            } => {
                let authority = session.authority();
                let caller = authority.admit_caller(session, text, assertion.as_ref())?;
                authority
                    .admit_items(session, e, items, params, &caller, text)
                    .map(drop)
            }
            Readmission::Registered {
                binding,
                ordinal,
                assertion,
            } => binding.recheck(e, *ordinal, assertion.as_ref()),
        }
    }
}

impl<R: Runtime> Engine<R> {
    /// Record, in a transaction of its own, that `role` running for `caller` was refused the call
    /// of `origin`, unless a mark stops the row.
    pub(in super::super) fn audit_denied(
        &self,
        role: &str,
        caller: &Val,
        origin: &CallOrigin,
    ) -> Result<(), Error> {
        self.audit_unless_marked(|| {
            store::run(self, true, &mut |e, _, id, changes| {
                Tables::new(&e.schema, id).audit_denied(e, changes, role, caller, origin)
            })
        })
    }

    /// Record the call `call`, in a transaction of its own, unless a mark stops the row.
    pub(super) fn audit_call(&self, call: &CallRow<'_>) -> Result<(), Error> {
        self.audit_unless_marked(|| {
            let seq = self.audit_seq()?;
            store::run(self, true, &mut |e, _, id, changes| {
                let t = Tables::new(&e.schema, id);
                changes.insert(t.audit_relation(), t.call_row(e, seq, call)?);
                Ok(())
            })
        })
    }

    /// Run `write`, the row of a call that changed nothing. A mark on the audit, found before or
    /// by `write`, leaves the row unwritten and the call's answer standing.
    fn audit_unless_marked(&self, write: impl FnOnce() -> Result<(), Error>) -> Result<(), Error> {
        let _open = self.enter()?;
        if self.audit_unwritable().is_some() {
            return Ok(());
        }
        write().or_else(|err| match self.audit_unwritable() {
            Some(_) => Ok(()),
            None => Err(err),
        })
    }

    /// Returns the failure of a change that must record a row of `catalog::audit`, while a mark
    /// on the database, or on a constructor or index of the audit, stops that row.
    pub(super) fn audit_unwritable(&self) -> Option<Error> {
        let audit = Tables::new(&self.schema, TxnId(0)).audit_relation();
        let marked = self.db.admit_relation(audit).err()?;
        Some(self.write_error(TxnId(0), marked))
    }

    /// Add the row of call `call`, the transaction `id`, to `tx` before its commit.
    pub(super) fn audit_call_in(
        &self,
        tx: &Txn,
        id: TxnId,
        call: &CallRow<'_>,
    ) -> Result<(), Error> {
        let seq = self.audit_seq()?;
        let t = Tables::new(&self.schema, id);
        let mut changes = Changes::default();
        changes.insert(t.audit_relation(), t.call_row(self, seq, call)?);
        changes
            .apply(self, tx)
            .map_err(|error| self.write_error(id, error))
    }

    /// Returns the next audit sequence number, read outside any transaction that adds a row so
    /// that concurrent calls do not conflict over it.
    pub(super) fn audit_seq(&self) -> Result<i64, Error> {
        if self.audit_seeded.load(Ordering::Acquire) {
            let audit = Tables::new(&self.schema, TxnId(0)).audit_relation();
            return self
                .db
                .deployment_id(audit, 1)
                .map_err(|error| self.write_error(TxnId(0), error));
        }
        let seq = store::run(self, false, &mut |e, tx, id, _| {
            let t = Tables::new(&e.schema, id);
            t.next_id(e, tx, t.audit_relation())
        })?;
        self.audit_seeded.store(true, Ordering::Release);
        Ok(seq)
    }

    /// Replace `old` of the relation `bundles` by `new` and record that `who` retired `bundle`, in
    /// a transaction of its own.
    pub(in super::super) fn retire_row(
        &self,
        bundles: RelId,
        (old, new): (Val, Val),
        who: (&str, &Val),
        bundle: &str,
    ) -> Result<(), Error> {
        store::run(self, true, &mut |e, _, id, changes| {
            changes.delete(bundles, old.clone());
            changes.insert(bundles, new.clone());
            Tables::new(&e.schema, id).audit_retire(e, changes, who, bundle)
        })
    }
}

/// Returns the BLAKE3-256 hash of `text`.
pub(super) fn text_hash(text: &str) -> [u8; 32] {
    *blake3::hash(text.as_bytes()).as_bytes()
}
