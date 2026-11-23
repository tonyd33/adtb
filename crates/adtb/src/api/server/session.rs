use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use adtb_lang::types::RelId;

use crate::exec::{Grant, Val};
use crate::runtime::Runtime;
use crate::write::Txn;

use super::super::bundle::BundleWriter;
use super::super::{Engine, Error, RejectReason, TxnId};
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
            self.derive(session, &Tables::new(e.schema(), id), e, tx, assertion)
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
            if let Some(quarantined) = e.refusal() {
                return Err(quarantined);
            }
            e.audit_denied(&session.name, caller, origin)?;
        }
        got
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
                Tables::new(e.schema(), id).audit_denied(e, changes, role, caller, origin)
            })
        })
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
                let t = Tables::new(e.schema(), id);
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
        let audit = Tables::new(self.schema(), TxnId(0)).audit_relation();
        let marked = self.db.admit_relation(audit).err()?;
        Some(self.write_error(TxnId(0), marked))
    }

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
        let t = Tables::new(self.schema(), id);
        let mut changes = Changes::default();
        changes.insert(t.audit_relation(), t.call_row(self, seq, call)?);
        changes
            .apply(self, tx, grant)
            .map_err(|error| self.write_error(id, error))
    }

    /// Returns the next audit sequence number, read outside any transaction that adds a row so
    /// that concurrent calls do not conflict over it.
    pub(super) fn audit_seq(&self) -> Result<i64, Error> {
        if self.audit_seeded.load(Ordering::Acquire) {
            let audit = Tables::new(self.schema(), TxnId(0)).audit_relation();
            return self
                .db
                .deployment_id(audit, 1)
                .map_err(|error| self.write_error(TxnId(0), error));
        }
        let seq = store::run(self, false, &mut |e, tx, id, _| {
            let t = Tables::new(e.schema(), id);
            t.next_seq(e, tx)
        })?;
        self.audit_seeded.store(true, Ordering::Release);
        Ok(seq)
    }
}

impl<R: Runtime> BundleWriter<'_, R> {
    /// Replace `old` of the relation `bundles` by `new` and record that `who` retired `bundle`, in
    /// a transaction of its own.
    pub(in super::super) fn retire_row(
        &self,
        bundles: RelId,
        (old, new): (Val, Val),
        who: (&str, &Val),
        bundle: &str,
    ) -> Result<(), Error> {
        store::run(self.engine(), true, &mut |e, _, id, changes| {
            changes.delete(bundles, old.clone());
            changes.insert(bundles, new.clone());
            Tables::new(e.schema(), id).audit_retire(e, changes, who, bundle)
        })
    }
}

/// Returns the BLAKE3-256 hash of `text`.
pub(super) fn text_hash(text: &str) -> [u8; 32] {
    *blake3::hash(text.as_bytes()).as_bytes()
}
