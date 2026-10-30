mod call;
mod initialize;
mod register;
mod result;
mod session;
mod store;
mod tables;
mod task;

use std::collections::BTreeSet;
use std::sync::{Arc, Weak};

use adtb_lang::ast::{AdhocItem, OperatorKind, Revoke};
use adtb_lang::evolve::{GrantRow, Source, closes_option_cycle, deployment_revoke};
use adtb_lang::lex::{Tok, lex};
use adtb_lang::parse::{Options, parse_adhoc};
use adtb_lang::schema::Schema;
use adtb_lang::span::Diag;

use crate::runtime::Runtime;

use super::{Database, Engine, Error, OWNER_ROLE, RejectReason, TxnId};
use store::Changes;
use tables::{Grant, Role, Tables};

pub use result::{Completion, Format, Layout, ResultBatch};
pub use session::RoleSession;
pub use task::{Observation, Task, TaskOptions};

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

    /// Drop a role and its deployment grants, credentials and peer mappings. Refuse a role the
    /// schema log names, or that owns a family or a bundle that is not retired.
    pub fn drop_role(&self, name: &str) -> Result<(), Error> {
        let name = role_name(name)?;
        let engine = self.engine.upgrade().ok_or_else(Error::closed)?;
        let _writer = engine.bundle_writer.lock(&engine.rt);
        self.access(true, |e, tx, id, changes| {
            let t = Tables::new(&e.schema, id);
            let role = t.role(e, tx, &name)?;
            if name == OWNER_ROLE || e.role_mentions.contains(&name) {
                return Err(denied("the role is required by the schema log"));
            }
            if let Some(why) = e.ownership(role.id.0) {
                return Err(denied(why));
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

    /// Grant one capability as the schema owner, retaining an existing grant option.
    pub fn grant_capability(
        &self,
        name: &str,
        capability: Capability,
        grant_option: bool,
    ) -> Result<(), Error> {
        self.give(OWNER_ROLE, name, capability, grant_option, |_, _, _, _| {
            Ok(())
        })
    }

    /// Grant one capability as `session`'s role, retaining a grant option it gave before.
    ///
    /// Fail with `Denied` unless the role holds `schema`, or the capability with grant option, and
    /// with `Invalid` when the grant with grant option would close a cycle of grant options.
    pub fn grant_capability_as(
        &self,
        session: &RoleSession<R>,
        name: &str,
        capability: Capability,
        grant_option: bool,
    ) -> Result<(), Error> {
        self.give(
            &session.name,
            name,
            capability,
            grant_option,
            |e, tx, t, grants| {
                self.checked(session, t, e, tx)?;
                require(grants, session, Capability::Schema, false)
                    .or_else(|_| require(grants, session, capability, true))
            },
        )
    }

    fn give(
        &self,
        grantor: &str,
        name: &str,
        capability: Capability,
        grant_option: bool,
        allowed: impl Fn(&Engine<R>, &crate::write::Txn, &Tables<'_>, &[Grant]) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let name = role_name(name)?;
        self.access(true, |e, tx, id, changes| {
            let t = Tables::new(&e.schema, id);
            let grants = t.grants(e, tx)?;
            allowed(e, tx, &t, &grants)?;
            t.role(e, tx, &name)?;
            if grant_option && closes_cycle(e, &grants, grantor, &name, capability) {
                return Err(Error::invalid(format!(
                    "`{}` with grant option from `{grantor}` to `{name}` would close a cycle of grant options",
                    capability.name()
                )));
            }
            let existing = grants
                .iter()
                .find(|g| g.capability(capability, &name) && g.grantor == grantor);
            let (id, effective_option) = match existing {
                Some(g) => {
                    changes.delete(t.grants, g.value.clone());
                    (g.id, grant_option || g.grant_option)
                }
                None => (t.next_id(e, tx, t.grants)?, grant_option),
            };
            changes.insert(
                t.grants,
                t.grant_value(id, &name, capability, grantor, effective_option),
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

    /// Revoke one capability as the schema owner, deleting every grantor's row.
    pub fn revoke_capability(&self, name: &str, capability: Capability) -> Result<(), Error> {
        let name = role_name(name)?;
        self.revoke_as(
            OWNER_ROLE,
            &format!("revoke {} from {name}", capability.name()),
            |_, _, _| Ok(()),
        )
    }

    /// Run the deployment revoke statement `text` as `session`'s role.
    ///
    /// Delete the matching rows the role granted, or every grantor's for the schema owner. Fail
    /// with `Invalid`, changing nothing, naming the revoke when a privilege matches no row the
    /// role may delete, or naming the rows when a deleted row held a grant option that rows of
    /// its grantee rely on and no option left to the grantee covers.
    pub fn revoke(&self, session: &RoleSession<R>, text: &str) -> Result<(), Error> {
        self.revoke_as(&session.name, text, |e, tx, t| {
            self.checked(session, t, e, tx)
        })
    }

    fn revoke_as(
        &self,
        issuer: &str,
        text: &str,
        allowed: impl Fn(&Engine<R>, &crate::write::Txn, &Tables<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let statement = revoke_statement(text)?;
        self.access(true, |e, tx, id, changes| {
            let t = Tables::new(&e.schema, id);
            allowed(e, tx, &t)?;
            let grants = t.grants(e, tx)?;
            let mut deleted: BTreeSet<i64> = BTreeSet::new();
            for p in &statement.privileges {
                let revoke = deployment_revoke(&e.schema, text, p)
                    .map_err(|ds| Error::invalid(diagnosed(&ds)))?;
                let matching: Vec<i64> = grants
                    .iter()
                    .filter(|g| {
                        g.kind == "Deployment"
                            && g.grantee == statement.from.text
                            && (issuer == OWNER_ROLE || g.grantor == issuer)
                            && revoke.matches(&e.schema, &g.privilege, g.target.as_deref())
                    })
                    .map(|g| g.id)
                    .collect();
                if matching.is_empty() {
                    return Err(Error::invalid(format!(
                        "`{text}` matches no grant `{issuer}` may revoke"
                    )));
                }
                deleted.extend(matching);
            }
            let gone = || grants.iter().filter(|g| deleted.contains(&g.id));
            let orphans = orphans(&e.schema, &grants, &deleted);
            if !orphans.is_empty() {
                return Err(Error::invalid(format!(
                    "`{text}` takes a grant option these grants rely on: {}",
                    orphans.join(", ")
                )));
            }
            for g in gone() {
                changes.delete(t.grants, g.value.clone());
            }
            t.audit(e, tx, changes, text.to_owned())
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

/// Returns each row that a deleted row's grant option supported and that no remaining option of
/// its grantor from another role covers, printed.
fn orphans(schema: &Schema, grants: &[Grant], deleted: &BTreeSet<i64>) -> Vec<String> {
    let (gone, left): (Vec<&Grant>, Vec<&Grant>) =
        grants.iter().partition(|g| deleted.contains(&g.id));
    left.iter()
        .filter(|o| {
            let row = o.row();
            gone.iter().any(|g| g.row().supports(schema, &row))
                && !left.iter().any(|g| g.row().keeps(schema, &row))
        })
        .map(|o| {
            format!(
                "`{}` on {:?} given by `{}` to `{}`",
                o.privilege, o.target, o.grantor, o.grantee
            )
        })
        .collect()
}

fn closes_cycle<R: Runtime>(
    e: &Engine<R>,
    grants: &[Grant],
    grantor: &str,
    grantee: &str,
    capability: Capability,
) -> bool {
    let rows: Vec<GrantRow<'_>> = grants.iter().map(Grant::row).collect();
    let new = GrantRow {
        grantor,
        grantee,
        privilege: capability.name(),
        target: None,
        grant_option: true,
        dormant: false,
    };
    closes_option_cycle(&e.schema, &rows, new)
}

fn diagnosed(diags: &[Diag]) -> String {
    let lines: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
    lines.join("; ")
}

fn revoke_statement(text: &str) -> Result<Revoke, Error> {
    let items = parse_adhoc(text, Options::default())
        .ok()
        .map_err(|ds| Error::invalid(diagnosed(&ds)))?;
    let mut items = items.into_iter();
    match (items.next(), items.next()) {
        (Some(AdhocItem::Revoke(r)), None) => Ok(r),
        _ => Err(Error::invalid(format!("`{text}` is not one revoke"))),
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
