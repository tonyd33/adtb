use std::sync::Arc;

use crate::catalog::Origin;
use crate::runtime::Runtime;

use super::super::bundle::{self, Claim, Registrant, Stored};
use super::super::registered::{self, Checked, Over, Registration};
use super::super::text::DynRows;
use super::super::{Engine, Error, OWNER_ID};
use super::tables::Tables;
use super::{RoleAuthority, RoleSession, denied};

impl<R: Runtime> RoleAuthority<R> {
    /// Register the bundle file `bundle` as the role of `session`. Returns its row of
    /// `catalog::bundles`.
    ///
    /// Postconditions:
    /// - Every check of registration passes, or the registration is refused and changes
    ///   nothing. A bundle registered before registers again only as its owner or the schema
    ///   owner. The role the bundle runs as holds with grant option every relation grant
    ///   what the bundle runs uses, unless that role is the schema owner; and the bundle's
    ///   family has no owner, or the session's role owns it, or is the schema owner.
    /// - A bundle with no successful registration runs as the session's role; one registered
    ///   before keeps its owner.
    /// - The first successful registration of a family makes the session's role its owner,
    ///   durably, with the bundle.
    pub fn register(
        &self,
        session: &RoleSession<R>,
        bundle: &Registration,
    ) -> Result<DynRows, Error> {
        let e = self.engine.upgrade().ok_or_else(Error::closed)?;
        let _open = e.enter()?;
        let _writer = e.bundle_writer.lock(&e.rt);
        let stored = e.deployed.lock(&e.rt).clone();
        let (checked, claim) = decide(&e, session, bundle, &stored)?;
        let family = &claim.row.header.family;
        let family_owner = stored.deployed.families.get(family).copied();
        drop(stored);
        self.access(true, |e, tx, id, changes| {
            let t = Tables::new(&e.schema, id);
            self.checked(session, &t, e, tx)?;
            let by = session.id.0;
            let roles = t.role_names(e, tx)?;
            let bundle = &claim.row.header.name;
            let name = |id, owned: &str| registered::role_name(&roles, id, owned);
            if claim.owned && claim.owner != by && by != OWNER_ID {
                let owner = name(claim.owner, &format!("bundle {bundle}"))?;
                return Err(denied(format!("bundle {bundle} is owned by role {owner}")));
            }
            if let Some(owner) = family_owner
                && owner != by
                && by != OWNER_ID
            {
                let owner = name(owner, &format!("family {family}"))?;
                return Err(denied(format!("family {family} is owned by role {owner}")));
            }
            if claim.owner != OWNER_ID {
                let runs_as = name(claim.owner, &format!("bundle {bundle}"))?;
                let lacks = registered::lacking(&checked, &e.source.grants, &runs_as)?;
                if !lacks.is_empty() {
                    return Err(denied(lacks.join("\n")));
                }
            }
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
            e.deployment
                .insert(&e.pager, x, &claim.facts)
                .map_err(Error::of_catalog_write)
        })?;
        serve(&e, checked, &claim);
        Ok(e.bundle_rows(&claim.row))
    }
}

/// Returns the checked registration of `bundle` over `stored` as the role of `session`, and
/// its writes.
fn decide<R: Runtime>(
    e: &Engine<R>,
    session: &RoleSession<R>,
    bundle: &Registration,
    stored: &Stored,
) -> Result<(Checked, Claim), Error> {
    if bundle.header().is_none() {
        return Err(Error::invalid(format!(
            "`{}` is no bundle file; only a bundle registers through a role session",
            bundle.name()
        )));
    }
    let now = e.clock();
    let s = *e.settings.lock(&e.rt);
    let over = Over {
        source: &e.source,
        schema: &e.schema,
        program: &e.program,
    };
    let checked = registered::registration(&over, bundle, stored, now, &s)?;
    let passed = checked
        .passed()
        .expect("a bundle's registration has a header");
    let by = Registrant {
        id: session.id.0,
        name: &session.name,
    };
    let claim = bundle::claim(stored, &passed, by, Origin::Command, now, &s);
    Ok((checked, claim))
}

impl<R: Runtime> Engine<R> {
    /// Returns why the role of id `id` owns deployment state: a family, or a bundle that is not
    /// retired.
    pub(super) fn ownership(&self, id: i64) -> Option<String> {
        let deployed = self.deployed.lock(&self.rt);
        let families = &deployed.deployed.families;
        if let Some((family, _)) = families.iter().find(|(_, o)| **o == id) {
            return Some(format!("the role owns family {family}"));
        }
        deployed
            .bundles
            .values()
            .find(|r| deployed.owner_of(r) == id && !r.retired())
            .map(|r| {
                format!(
                    "the role owns bundle {}, which is not retired",
                    r.header.name
                )
            })
    }
}

/// Serve `checked`, whose claim has committed.
fn serve<R: Runtime>(e: &Engine<R>, checked: Checked, claim: &Claim) {
    claim.record(Arc::make_mut(&mut e.deployed.lock(&e.rt)));
    let mut explained = (*e.explained()).clone();
    checked.explained(&mut explained);
    *e.bundles.lock(&e.rt) = Arc::new(explained);
    let mut served = e.registered.lock(&e.rt);
    served.retain(|c| c.name() != checked.name());
    served.push(Arc::new(checked));
}
