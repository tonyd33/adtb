use std::sync::{Arc, Weak};

use adtb_lang::types::RelId;

use crate::exec::{Grant, Val};
use crate::runtime::Runtime;
use crate::write::Txn;

use super::super::bundle::BundleWriter;
use super::super::{Engine, Entered, Error, RejectReason, TxnId};
use super::store::{self, Changes};
use super::tables::{CallOrigin, CallRow, RoleId, Tables, service};
use super::{Capability, RoleAuthority, require};

#[derive(Clone, Debug)]
pub struct RoleSession<R: Runtime = adtb_sys::os::OsRuntime> {
    pub(super) serial: u64,
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
            self.derive(
                session,
                &Tables::new(e.schema(), id, None),
                e,
                tx,
                assertion,
            )
        })
    }

    pub(super) fn derive(
        &self,
        session: &RoleSession<R>,
        t: &Tables<'_>,
        e: &Entered<'_, R>,
        tx: &Txn,
        assertion: Option<&Val>,
    ) -> Result<Val, Error> {
        self.checked(session, t, e, tx)?;
        let Some(assertion) = assertion else {
            return Ok(service(e.schema(), &session.name));
        };
        require(&t.grants(e, tx)?, session, Capability::Assert, false)?;
        e.principal(Some(assertion))
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
            let e = &e.enter()?;
            if let Some(quarantined) = e.refusal() {
                return Err(quarantined);
            }
            e.audit_denied(&session.name, caller, origin, None)?;
        }
        got
    }
}

impl<R: Runtime> BundleWriter<'_, R> {
    /// Replace `old` of the relation `bundles` by `new` and record that `who` retired `bundle`, in
    /// a transaction of its own charged to the call's `grant`.
    pub(in super::super) fn retire_row(
        &self,
        e: &Entered<'_, R>,
        bundles: RelId,
        (old, new): (Val, Val),
        (who, bundle): ((&str, &Val), &str),
        grant: Option<&Arc<Grant>>,
    ) -> Result<(), Error> {
        store::run_under(e, true, grant, &mut |e, _, id, changes| {
            changes.delete(bundles, old.clone());
            changes.insert(bundles, new.clone());
            Tables::new(e.schema(), id, grant).audit_retire(e, changes, who, bundle)
        })
    }
}

/// Returns the BLAKE3-256 hash of `text`.
pub(super) fn text_hash(text: &str) -> [u8; 32] {
    *blake3::hash(text.as_bytes()).as_bytes()
}

impl<R: Runtime> Entered<'_, R> {
    /// Add the row of call `call`, the transaction `id`, to `tx` before its commit, charged to the
    /// call's `grant`.
    pub(super) fn audit_call_in(
        &self,
        tx: &Txn,
        id: TxnId,
        call: &CallRow<'_>,
        grant: Option<&Arc<Grant>>,
    ) -> Result<(), Error> {
        let seq = self.audit_seq()?;
        let t = Tables::new(self.schema(), id, grant);
        let mut changes = Changes::default();
        changes.insert(t.audit_relation(), t.call_row(self, seq, call)?);
        changes
            .apply(self, tx, grant)
            .map_err(|error| self.write_error(id, error))
    }

    /// Returns the next audit sequence number, drawn outside any transaction that adds a row so
    /// that concurrent calls do not conflict over it.
    pub(super) fn audit_seq(&self) -> Result<i64, Error> {
        let audit = Tables::new(self.schema(), TxnId(0), None).audit_relation();
        self.db
            .deployment_id(audit, 1)
            .map_err(|error| self.write_error(TxnId(0), error))
    }

    /// Returns the failure of a change that must record a row of `catalog::audit`, while a mark
    /// on the database, or on a constructor or index of the audit, stops that row.
    pub(super) fn audit_unwritable(&self) -> Option<Error> {
        let audit = Tables::new(self.schema(), TxnId(0), None).audit_relation();
        let marked = self.db.admit_relation(audit).err()?;
        Some(self.write_error(TxnId(0), marked))
    }

    /// Record the call `call`, in a transaction of its own charged to the call's `grant`, unless
    /// a mark stops the row.
    pub(super) fn audit_call(
        &self,
        call: &CallRow<'_>,
        grant: Option<&Arc<Grant>>,
    ) -> Result<(), Error> {
        self.audit_unless_marked(|| {
            let seq = self.audit_seq()?;
            store::run_under(self, true, grant, &mut |e, _, id, changes| {
                let t = Tables::new(e.schema(), id, grant);
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

    /// Record, in a transaction of its own charged to the call's `grant`, that `role` running for
    /// `caller` was refused the call of `origin`, unless a mark stops the row.
    pub(in super::super) fn audit_denied(
        &self,
        role: &str,
        caller: &Val,
        origin: &CallOrigin,
        grant: Option<&Arc<Grant>>,
    ) -> Result<(), Error> {
        self.audit_unless_marked(|| {
            store::run_under(self, true, grant, &mut |e, _, id, changes| {
                Tables::new(e.schema(), id, grant).audit_denied(e, changes, role, caller, origin)
            })
        })
    }
}
