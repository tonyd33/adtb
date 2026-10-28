mod initialize;
mod session;
mod store;
mod tables;

use std::collections::BTreeSet;
use std::sync::{Arc, Weak};

use adtb_lang::ast::OperatorKind;
use adtb_lang::evolve::Source;
use adtb_lang::lex::{Tok, lex};

use crate::runtime::Runtime;

use super::{Database, Engine, Error, OWNER_ROLE, RejectReason, TxnId};
use store::Changes;
use tables::{Grant, Role, Tables};

pub use session::RoleSession;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    Adhoc,
    Assert,
    Schema,
    Monitor,
    Cancel,
    Maintain,
    Configure,
    ReadFiles,
    WriteFiles,
}

impl Capability {
    pub fn name(self) -> &'static str {
        match self {
            Self::Adhoc => "adhoc",
            Self::Assert => "assert",
            Self::Schema => "schema",
            Self::Monitor => "monitor",
            Self::Cancel => "cancel",
            Self::Maintain => "maintain",
            Self::Configure => "configure",
            Self::ReadFiles => "read_files",
            Self::WriteFiles => "write_files",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RoleAuthority<R: Runtime = adtb_sys::os::OsRuntime> {
    engine: Weak<Engine<R>>,
}

impl<R: Runtime> Database<R> {
    /// Returns the trusted embedded owner's role administration provider.
    pub fn role_authority(&self) -> RoleAuthority<R> {
        RoleAuthority {
            engine: Arc::downgrade(&self.engine),
        }
    }
}

impl<R: Runtime> RoleAuthority<R> {
    /// Issue a session for a role authenticated by the trusted caller.
    ///
    /// Precondition: establish transport authentication before issuing a network session.
    pub fn session(&self, name: &str) -> Result<RoleSession<R>, Error> {
        let name = role_name(name)?;
        self.access(false, |e, tx, id, _| {
            let t = Tables::new(&e.schema, id);
            let role = t.role(e, tx, &name)?;
            Ok(RoleSession {
                engine: self.engine.clone(),
                id: role.id,
                name: role.name,
            })
        })
    }

    /// Create a role without credentials or grants.
    pub fn create_role(&self, name: &str) -> Result<(), Error> {
        let name = role_name(name)?;
        self.access(true, |e, tx, id, changes| {
            let t = Tables::new(&e.schema, id);
            let roles = t.roles(e, tx)?;
            if roles.iter().any(|r| r.name == name) {
                return Err(Error::invalid(format!("role `{name}` already exists")));
            }
            let id = t.next_id(e, tx, t.roles)?;
            let role = Role {
                id: tables::RoleId(id),
                name: name.to_owned(),
                created: e.clock(),
            };
            changes.insert(t.roles, t.role_value(&role));
            t.audit(e, tx, changes, format!("create role {name}"))
        })
    }

    /// Drop a role and its deployment grants, credentials and peer mappings.
    pub fn drop_role(&self, name: &str) -> Result<(), Error> {
        let name = role_name(name)?;
        self.access(true, |e, tx, id, changes| {
            let t = Tables::new(&e.schema, id);
            let role = t.role(e, tx, &name)?;
            if name == OWNER_ROLE
                || e.role_mentions.contains(&name)
                || t.owns_bundle(e, tx, &name)?
            {
                return Err(denied(
                    "the role is required by the schema log or bundle ownership",
                ));
            }
            changes.delete(t.roles, t.role_value(&role));
            for grant in t.grants(e, tx)? {
                if grant.grantee == name || grant.grantor == name {
                    changes.delete(t.grants, grant.value);
                }
            }
            for row in t.read(e, tx, t.credentials)? {
                if t.integer(t.credentials, &row, "role")? == role.id.0 {
                    changes.delete(t.credentials, row);
                }
            }
            t.remove_peer_mappings(e, tx, changes, &name)?;
            t.audit(e, tx, changes, format!("drop role {name}"))
        })
    }

    /// Grant one capability, retaining an existing grant option.
    pub fn grant_capability(
        &self,
        name: &str,
        capability: Capability,
        grant_option: bool,
    ) -> Result<(), Error> {
        let name = role_name(name)?;
        self.access(true, |e, tx, id, changes| {
            let t = Tables::new(&e.schema, id);
            t.role(e, tx, &name)?;
            let grants = t.grants(e, tx)?;
            let existing = grants
                .iter()
                .find(|g| g.capability(capability, &name) && g.grantor == OWNER_ROLE);
            let (id, effective_option) = match existing {
                Some(g) => {
                    changes.delete(t.grants, g.value.clone());
                    (g.id, grant_option || g.grant_option)
                }
                None => (t.next_id(e, tx, t.grants)?, grant_option),
            };
            changes.insert(
                t.grants,
                t.grant_value(id, &name, capability, effective_option),
            );
            let option = if grant_option {
                " with grant option"
            } else {
                ""
            };
            t.audit(
                e,
                tx,
                changes,
                format!("grant {} to {name}{option}", capability.name()),
            )
        })
    }

    /// Revoke every deployment grant of one capability from the role.
    pub fn revoke_capability(&self, name: &str, capability: Capability) -> Result<(), Error> {
        let name = role_name(name)?;
        self.access(true, |e, tx, id, changes| {
            let t = Tables::new(&e.schema, id);
            t.role(e, tx, &name)?;
            for grant in t.grants(e, tx)? {
                if grant.capability(capability, &name) {
                    changes.delete(t.grants, grant.value);
                }
            }
            t.audit(
                e,
                tx,
                changes,
                format!("revoke {} from {name}", capability.name()),
            )
        })
    }

    /// Check a session's current capability and optional right to grant it onward.
    pub fn require(
        &self,
        session: &RoleSession<R>,
        capability: Capability,
        grant_option: bool,
    ) -> Result<(), Error> {
        self.access(false, |e, tx, id, _| {
            let t = Tables::new(&e.schema, id);
            self.checked(session, &t, e, tx)?;
            require(&t.grants(e, tx)?, session, capability, grant_option)
        })
    }

    fn access<T>(
        &self,
        write: bool,
        mut body: impl FnMut(&Engine<R>, &crate::write::Txn, TxnId, &mut Changes) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let engine = self.engine.upgrade().ok_or_else(Error::closed)?;
        let _open = engine.enter()?;
        store::run(&engine, write, &mut body)
    }

    fn checked(
        &self,
        session: &RoleSession<R>,
        t: &Tables<'_>,
        e: &Engine<R>,
        tx: &crate::write::Txn,
    ) -> Result<(), Error> {
        if !Weak::ptr_eq(&self.engine, &session.engine) {
            return Err(denied("the role session belongs to another database"));
        }
        if t.role(e, tx, &session.name)?.id != session.id {
            return Err(denied(
                "the role session no longer names its authenticated role",
            ));
        }
        Ok(())
    }
}

fn require<R: Runtime>(
    grants: &[Grant],
    session: &RoleSession<R>,
    capability: Capability,
    grant_option: bool,
) -> Result<(), Error> {
    if session.name == OWNER_ROLE
        || grants
            .iter()
            .any(|g| g.capability(capability, &session.name) && (!grant_option || g.grant_option))
    {
        Ok(())
    } else {
        Err(denied(format!(
            "role `{}` lacks `{}`",
            session.name,
            capability.name()
        )))
    }
}

fn denied(detail: impl Into<String>) -> Error {
    Error::rejected(RejectReason::Denied {
        detail: detail.into(),
    })
}

fn role_name(name: &str) -> Result<String, Error> {
    let name = name.to_owned();
    let (tokens, errors) = lex(&name);
    if errors.is_empty()
        && tokens.len() == 2
        && tokens[0].tok == Tok::Lower
        && tokens[0].span.range() == (0..name.len())
    {
        Ok(name)
    } else {
        Err(Error::invalid("a role name is one lowercase identifier"))
    }
}

pub(super) fn mentioned(source: &Source) -> BTreeSet<String> {
    source
        .log
        .iter()
        .flat_map(|m| &m.operators)
        .filter_map(|op| match &op.kind {
            OperatorKind::Grant(g) => Some(g.to.text.clone()),
            _ => None,
        })
        .collect()
}

pub(super) use initialize::initialize;
