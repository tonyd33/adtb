use std::sync::Arc;

use crate::runtime::Runtime;

use super::super::bundle::{self, Claim};
use super::super::registered::{self, Checked, Over, Registration};
use super::super::text::DynRows;
use super::super::{Engine, Error, OWNER_ID, OWNER_ROLE, RejectReason};
use super::tables::Tables;
use super::{RoleAuthority, RoleSession, denied};

impl<R: Runtime> RoleAuthority<R> {
    /// Register the bundle file `bundle` as the role of `session`, which owns it. Returns its row
    /// of `catalog::bundles`.
    ///
    /// Postconditions:
    /// - Every check of registration passes, or the registration is refused and changes
    ///   nothing. The role holds with grant option every relation grant what the bundle runs
    ///   uses, and the bundle's family has no owner, or the role owns it, or the role is the
    ///   schema owner.
    /// - The first successful registration of a family makes the role its owner, durably, with
    ///   the bundle.
    pub fn register(
        &self,
        session: &RoleSession<R>,
        bundle: &Registration,
    ) -> Result<DynRows, Error> {
        let e = self.engine.upgrade().ok_or_else(Error::closed)?;
        let _open = e.enter()?;
        let _writer = e.bundle_writer.lock(&e.rt);
        let (checked, claim) = decide(&e, session, bundle)?;
        self.access(true, |e, tx, id, changes| {
            let t = Tables::new(&e.schema, id);
            self.checked(session, &t, e, tx)?;
            claim.rows(&e.schema, &mut |relation, insert, row| {
                if insert {
                    changes.insert(relation, row);
                } else {
                    changes.delete(relation, row);
                }
            });
            let x = tx
                .xid()
                .expect("a write transaction of the write path has an id");
            e.deployment.insert(&e.pager, x, &claim.facts).map_err(|c| {
                Error::rejected(RejectReason::Internal {
                    what: format!("{c:?}"),
                })
            })
        })?;
        serve(&e, checked, &claim);
        Ok(e.bundle_rows(&claim.row))
    }
}

/// Returns the checked registration of `bundle` as the role of `session`, and its writes.
fn decide<R: Runtime>(
    e: &Engine<R>,
    session: &RoleSession<R>,
    bundle: &Registration,
) -> Result<(Checked, Claim), Error> {
    if bundle.header().is_none() {
        return Err(Error::invalid(format!(
            "`{}` is no bundle file; only a bundle registers through a role session",
            bundle.name()
        )));
    }
    let now = e.clock();
    let s = *e.settings.lock(&e.rt);
    let mut stored = e.ledger.lock(&e.rt).clone();
    stored.bundles = e.deployed.lock(&e.rt).clone();
    let over = Over {
        source: &e.source,
        schema: &e.schema,
        program: &e.program,
    };
    let checked = registered::registration(&over, bundle, &stored, now, &s)?;
    let role = session.id.0;
    if role != OWNER_ID {
        let lacks = registered::lacking(&checked, &e.source.grants, &session.name)?;
        if !lacks.is_empty() {
            return Err(denied(lacks.join("\n")));
        }
    }
    let passed = checked
        .passed()
        .expect("a bundle's registration has a header");
    let family = &passed.header.family;
    if let Some(&owner) = stored.deployed.families.get(family)
        && owner != role
        && role != OWNER_ID
    {
        return Err(denied(format!(
            "family {family} is owned by role {}",
            owner_name(&stored, owner)
        )));
    }
    let claim = bundle::claim(&stored, &passed, &session.name, role, now, &s);
    Ok((checked, claim))
}

/// Returns the name of the role of id `id` as the bundles it registered record it.
fn owner_name(stored: &bundle::Stored, id: i64) -> String {
    if id == OWNER_ID {
        return OWNER_ROLE.to_owned();
    }
    stored
        .bundles
        .values()
        .find(|r| {
            stored
                .deployed
                .registered
                .get(&r.id)
                .is_some_and(|m| m.role == id)
        })
        .map_or_else(|| format!("#{id}"), |r| r.owner.clone())
}

/// Serve `checked`, whose claim has committed.
fn serve<R: Runtime>(e: &Engine<R>, checked: Checked, claim: &Claim) {
    claim.record(&mut e.ledger.lock(&e.rt));
    e.deployed
        .lock(&e.rt)
        .insert(claim.row.header.name.clone(), claim.row.clone());
    let mut explained = (*e.explained()).clone();
    checked.explained(&mut explained);
    *e.bundles.lock(&e.rt) = Arc::new(explained);
    let mut served = e.registered.lock(&e.rt);
    served.retain(|c| c.name() != checked.name());
    served.push(Arc::new(checked));
}
