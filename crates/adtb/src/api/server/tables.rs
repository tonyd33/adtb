use std::collections::BTreeMap;
use std::sync::Arc;

use adtb_lang::evolve::{DeployedGrant, GrantRow};
use adtb_lang::ids::{CatRelId, engine_ctor as k, engine_rel};
use adtb_lang::schema::{CtorRef, Schema};
use adtb_lang::textual::{Form, print};
use adtb_lang::types::{RelId, Ty};

use crate::catalog::codec::{
    Codec, Ctor, Decode, Field, Fields, GrantKind, RoleGrant, RoleRow, field,
};
use crate::catalog::{Rot, engine_relation, layout};
use crate::exec::Val;
use crate::runtime::Runtime;
use crate::write::{self, Txn};

use super::super::cause::Broken;
use super::super::error_value::NoValue;
use super::super::{Engine, Error, FatalReason, TxnId};
use super::store::Changes;
use super::{Capability, denied};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RoleId(pub(super) i64);

pub(super) struct Role {
    pub(super) id: RoleId,
    pub(super) name: String,
    pub(super) created: i64,
}

#[derive(Clone)]
pub(super) struct Grant {
    pub(super) id: i64,
    pub(super) grantee: String,
    pub(super) grantor: String,
    pub(super) grant_option: bool,
    pub(super) kind: GrantKind,
    pub(super) privilege: String,
    pub(super) target: Option<String>,
    pub(super) dormant: bool,
    pub(super) value: Val,
}

impl Grant {
    pub(super) fn row(&self) -> GrantRow<'_> {
        GrantRow {
            grantor: &self.grantor,
            grantee: &self.grantee,
            privilege: &self.privilege,
            target: self.target.as_deref(),
            grant_option: self.grant_option,
            dormant: self.dormant,
        }
    }

    /// Returns what the row gives its grantee, when it is a `Deployment` row.
    pub(super) fn deployed(&self) -> Option<DeployedGrant> {
        (self.kind == GrantKind::Deployment).then(|| DeployedGrant {
            grantee: self.grantee.clone(),
            privilege: self.privilege.clone(),
            target: self.target.clone(),
            grant_option: self.grant_option,
            dormant: self.dormant,
        })
    }

    /// Returns the row printed for a refusal.
    pub(super) fn printed(&self) -> String {
        let on = self
            .target
            .as_ref()
            .map_or_else(String::new, |t| format!(" on `{t}`"));
        format!(
            "`{}`{on} given by `{}` to `{}`",
            self.privilege, self.grantor, self.grantee
        )
    }

    pub(super) fn capability(&self, capability: Capability, name: &str) -> bool {
        self.kind == GrantKind::Deployment
            && self.privilege == capability.name()
            && self.target.is_none()
            && self.grantee == name
            && !self.dormant
    }
}

/// A deployment grant being given: the identity of its row.
pub(super) struct Given<'a> {
    pub(super) grantor: &'a str,
    pub(super) grantee: &'a str,
    pub(super) privilege: &'a str,
    pub(super) target: Option<&'a str>,
}

pub(super) struct Tables<'s> {
    schema: &'s Schema,
    call: TxnId,
    pub(super) roles: RelId,
    pub(super) grants: RelId,
    pub(super) credentials: RelId,
    peers: RelId,
    audit: RelId,
}

impl<'s> Tables<'s> {
    /// Returns the row of `id` a deployment grant holds.
    pub(super) fn deployment_grant(&self, id: i64, given: &Given<'_>, grant_option: bool) -> Grant {
        Grant {
            id,
            grantee: given.grantee.to_owned(),
            grantor: given.grantor.to_owned(),
            grant_option,
            kind: GrantKind::Deployment,
            privilege: given.privilege.to_owned(),
            target: given.target.map(str::to_owned),
            dormant: false,
            value: self.grant_value(
                id,
                given.grantee,
                given.privilege,
                given.target,
                given.grantor,
                grant_option,
            ),
        }
    }

    /// Hold the row of `given` in `rows`: the row already held gains the option, and otherwise a
    /// row is added.
    pub(super) fn give_row<R: Runtime>(
        &self,
        e: &Engine<R>,
        tx: &Txn,
        rows: &mut Vec<Grant>,
        given: &Given<'_>,
        grant_option: bool,
    ) -> Result<(), Error> {
        let held = rows.iter().position(|g| {
            g.kind == GrantKind::Deployment
                && !g.dormant
                && g.grantee == given.grantee
                && g.grantor == given.grantor
                && g.privilege == given.privilege
                && g.target.as_deref() == given.target
        });
        match held {
            Some(i) if rows[i].grant_option || !grant_option => {}
            Some(i) => rows[i] = self.deployment_grant(rows[i].id, given, true),
            None => {
                let id = self.next_grant_id(e, tx)?;
                rows.push(self.deployment_grant(id, given, grant_option));
            }
        }
        Ok(())
    }

    /// Record the difference between the rows `before` and `after` of `catalog::grants`.
    pub(super) fn write_rows(&self, changes: &mut Changes, before: &[Grant], after: &[Grant]) {
        for old in before {
            if !after.iter().any(|g| g.id == old.id && g.value == old.value) {
                changes.delete(self.grants, old.value.clone());
            }
        }
        for new in after {
            if !before
                .iter()
                .any(|g| g.id == new.id && g.value == new.value)
            {
                changes.insert(self.grants, new.value.clone());
            }
        }
    }

    pub(super) fn new(schema: &'s Schema, call: TxnId) -> Self {
        let relation = |id| engine_relation(schema, id);
        Tables {
            schema,
            call,
            roles: relation(engine_rel::ROLES),
            grants: relation(engine_rel::GRANTS),
            credentials: relation(engine_rel::CREDENTIALS),
            peers: relation(engine_rel::PEER_MAPPINGS),
            audit: relation(engine_rel::AUDIT),
        }
    }

    fn read<R: Runtime>(&self, e: &Engine<R>, tx: &Txn, r: RelId) -> Result<Vec<Val>, Error> {
        e.db.relation_rows(tx, r)
            .map_err(|error| e.write_error(self.call, error))
    }

    /// Returns each row of `codec`'s relation, read by `codec`, with its value.
    ///
    /// Fail with `Internal` when `codec` refuses a row, which the heap decoded by its declared type.
    fn typed<T: Decode, R: Runtime>(
        &self,
        e: &Engine<R>,
        tx: &Txn,
        codec: &Codec<T>,
    ) -> Result<Vec<(T, Val)>, Error> {
        self.read(e, tx, codec.relation())?
            .into_iter()
            .map(|v| {
                let row = codec.decode_val(self.schema, &v).map_err(|r: Rot| {
                    e.write_error(self.call, write::Broken::CatalogError(r.into()).into())
                })?;
                Ok((row, v))
            })
            .collect()
    }

    fn row(&self, relation: RelId, ctor: CtorRef, fields: Vec<(&str, Val)>) -> Val {
        let index = ctor.index();
        let mut values: BTreeMap<_, _> = fields.into_iter().collect();
        let fields = layout::row(self.schema, relation, index)
            .iter()
            .map(|(name, _)| {
                values
                    .remove(name.as_str())
                    .unwrap_or_else(|| panic!("the catalog row provides {name}"))
            })
            .collect();
        assert!(
            values.is_empty(),
            "a catalog row provides exactly its declared fields"
        );
        Val::ctor(index, fields)
    }

    pub(super) fn roles<R: Runtime>(&self, e: &Engine<R>, tx: &Txn) -> Result<Vec<Role>, Error> {
        let codec = Codec::<RoleRow>::new(self.schema);
        Ok(self
            .typed(e, tx, &codec)?
            .into_iter()
            .map(|(row, _)| {
                let (id, name, created) = row.into_parts();
                Role {
                    id: RoleId(id),
                    name,
                    created,
                }
            })
            .collect())
    }

    /// Returns the name of each role, by id.
    pub(super) fn role_names<R: Runtime>(
        &self,
        e: &Engine<R>,
        tx: &Txn,
    ) -> Result<BTreeMap<i64, String>, Error> {
        Ok(self
            .roles(e, tx)?
            .into_iter()
            .map(|r| (r.id.0, r.name))
            .collect())
    }

    pub(super) fn role<R: Runtime>(
        &self,
        e: &Engine<R>,
        tx: &Txn,
        name: &str,
    ) -> Result<Role, Error> {
        self.roles(e, tx)?
            .into_iter()
            .find(|r| r.name == name)
            .ok_or_else(|| denied("the authenticated role no longer exists"))
    }

    /// Fail with `Invalid`, naming `name`, when no role has that name.
    pub(super) fn grantee<R: Runtime>(
        &self,
        e: &Engine<R>,
        tx: &Txn,
        name: &str,
    ) -> Result<(), Error> {
        if self.roles(e, tx)?.iter().any(|r| r.name == name) {
            return Ok(());
        }
        Err(Error::invalid(format!(
            "the grantee `{name}` is not a role"
        )))
    }

    pub(super) fn role_value(&self, role: &Role) -> Val {
        self.row(
            self.roles,
            self.schema.engine_ctors().role(),
            vec![
                ("id", Val::Int(role.id.0)),
                ("name", Val::text(&role.name)),
                ("created", Val::Instant(role.created)),
            ],
        )
    }

    pub(super) fn grants<R: Runtime>(&self, e: &Engine<R>, tx: &Txn) -> Result<Vec<Grant>, Error> {
        let codec = Codec::<RoleGrant>::new(self.schema);
        Ok(self
            .typed(e, tx, &codec)?
            .into_iter()
            .map(|(g, value)| {
                let row = g.row();
                Grant {
                    id: g.id(),
                    grantee: row.grantee.to_owned(),
                    grantor: row.grantor.to_owned(),
                    grant_option: row.grant_option,
                    kind: g.kind(),
                    privilege: row.privilege.to_owned(),
                    target: row.target.map(str::to_owned),
                    dormant: row.dormant,
                    value,
                }
            })
            .collect())
    }

    pub(super) fn grant_value(
        &self,
        id: i64,
        role: &str,
        privilege: &str,
        target: Option<&str>,
        grantor: &str,
        grant_option: bool,
    ) -> Val {
        let kind = self.schema.engine_ctors().deployment().index();
        self.row(
            self.grants,
            self.schema.engine_ctors().role_grant(),
            vec![
                ("id", Val::Int(id)),
                ("kind", Val::ctor(kind, Vec::new())),
                ("privilege", Val::text(privilege)),
                (
                    "target",
                    target.map_or_else(Val::none, |t| Val::some(Val::text(t))),
                ),
                ("grantee", Val::text(role)),
                ("grantor", Val::text(grantor)),
                ("grant_option", Val::bool(grant_option)),
                ("dormant", Val::bool(false)),
            ],
        )
    }

    /// Returns a new id of `catalog::roles`.
    pub(super) fn next_role_id<R: Runtime>(&self, e: &Engine<R>, tx: &Txn) -> Result<i64, Error> {
        let highest = self.roles(e, tx)?.iter().map(|r| r.id.0).max();
        self.drawn(e, self.roles, highest)
    }

    /// Returns a new id of `catalog::grants`.
    pub(super) fn next_grant_id<R: Runtime>(&self, e: &Engine<R>, tx: &Txn) -> Result<i64, Error> {
        let highest = self.grants(e, tx)?.iter().map(|g| g.id).max();
        self.drawn(e, self.grants, highest)
    }

    /// Returns a new sequence number of `catalog::audit`.
    pub(super) fn next_seq<R: Runtime>(&self, e: &Engine<R>, tx: &Txn) -> Result<i64, Error> {
        let codec = Codec::<AuditRow>::new(self.schema);
        let rows = self.typed(e, tx, &codec)?;
        let highest = rows.iter().map(|(a, _)| a.seq).max();
        self.drawn(e, self.audit, highest)
    }

    /// Returns a new id of `relation` past `highest`, the highest it holds.
    fn drawn<R: Runtime>(
        &self,
        e: &Engine<R>,
        relation: RelId,
        highest: Option<i64>,
    ) -> Result<i64, Error> {
        let floor = highest
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| Error::invalid("the deployment identity range is exhausted"))?;
        e.db.deployment_id(relation, floor)
            .map_err(|error| e.write_error(self.call, error))
    }

    /// Returns the value of each row of `catalog::credentials` of role `role`.
    pub(super) fn credentials_of<R: Runtime>(
        &self,
        e: &Engine<R>,
        tx: &Txn,
        role: RoleId,
    ) -> Result<Vec<Val>, Error> {
        let codec = Codec::<CredentialRow>::new(self.schema);
        Ok(self
            .typed(e, tx, &codec)?
            .into_iter()
            .filter(|(c, _)| c.role == role.0)
            .map(|(_, v)| v)
            .collect())
    }

    pub(super) fn remove_peer_mappings<R: Runtime>(
        &self,
        e: &Engine<R>,
        tx: &Txn,
        changes: &mut Changes,
        name: &str,
    ) -> Result<(), Error> {
        let codec = Codec::<PeerRow>::new(self.schema);
        for (peer, row) in self.typed(e, tx, &codec)? {
            if peer.role == name {
                changes.delete(self.peers, row);
            }
        }
        Ok(())
    }

    /// Record that `role`, running as its own service, made the deployment change `statement`.
    pub(super) fn audit<R: Runtime>(
        &self,
        e: &Engine<R>,
        changes: &mut Changes,
        role: &str,
        statement: String,
    ) -> Result<(), Error> {
        let principal = service(self.schema, role);
        let event = ("statement", Val::text(&statement));
        self.audit_row(e, changes, "Grant", (role, &principal), event)
    }

    /// Record that `role`, running for `caller`, was refused the call of `origin`.
    pub(super) fn audit_denied<R: Runtime>(
        &self,
        e: &Engine<R>,
        changes: &mut Changes,
        role: &str,
        caller: &Val,
        origin: &CallOrigin,
    ) -> Result<(), Error> {
        let event = ("origin", self.origin(origin)?);
        self.audit_row(e, changes, "Denied", (role, caller), event)
    }

    /// Record that `role`, running for `caller`, retired `bundle`.
    pub(super) fn audit_retire<R: Runtime>(
        &self,
        e: &Engine<R>,
        changes: &mut Changes,
        (role, caller): (&str, &Val),
        bundle: &str,
    ) -> Result<(), Error> {
        let event = ("bundle", Val::text(bundle));
        self.audit_row(e, changes, "Retire", (role, caller), event)
    }

    /// Returns the `Call` row numbered `seq` of `call`.
    pub(super) fn call_row<R: Runtime>(
        &self,
        e: &Engine<R>,
        seq: i64,
        call: &CallRow<'_>,
    ) -> Result<Val, Error> {
        let fields = vec![
            ("origin", self.origin(call.origin)?),
            ("args", Val::Bytes(Arc::from(call.args.as_slice()))),
            ("outcome", self.outcome(call.failed)?),
        ];
        self.event_row(e, seq, "Call", (call.role, call.caller), fields)
    }

    fn origin(&self, origin: &CallOrigin) -> Result<Val, Error> {
        let def = self.schema.ty(self.schema.prelude().origin());
        let ctor = |name: &str| {
            def.ctor_index(name)
                .ok_or_else(|| malformed(format!("`Origin` declares `{name}`")))
        };
        Ok(match origin {
            CallOrigin::Registered { bundle, query } => Val::ctor(
                ctor("Registered")?,
                vec![Val::text(bundle), Val::Int(i64::from(*query))],
            ),
            CallOrigin::Function { name } => Val::ctor(ctor("Function")?, vec![Val::text(name)]),
            CallOrigin::AdHoc(text) => {
                Val::ctor(ctor("AdHoc")?, vec![Val::Bytes(Arc::from(text.as_slice()))])
            }
        })
    }

    fn outcome(&self, failed: Option<&Error>) -> Result<Val, Error> {
        let def = self.schema.ty(self.schema.prelude().outcome());
        let ctor = |name: &str| {
            def.ctor_index(name)
                .ok_or_else(|| malformed(format!("`Outcome` declares `{name}`")))
        };
        Ok(match failed {
            None => Val::ctor(ctor("Succeeded")?, Vec::new()),
            Some(error) => Val::ctor(ctor("Failed")?, vec![self.error_value(error)]),
        })
    }

    /// Returns `error` as a value of the prelude sum, or as `Internal` naming it when it holds a
    /// report.
    fn error_value(&self, error: &Error) -> Val {
        match error.prelude_value(self.schema) {
            Ok(value) => value,
            Err(NoValue::Report) => Error::internal(Broken::NoValue(error.clone()))
                .prelude_value(self.schema)
                .unwrap_or_else(|e| unreachable!("an `Internal` error holds no report: {e:?}")),
        }
    }

    fn audit_row<R: Runtime>(
        &self,
        e: &Engine<R>,
        changes: &mut Changes,
        ctor: &str,
        who: (&str, &Val),
        event: (&str, Val),
    ) -> Result<(), Error> {
        let seq = e.audit_seq()?;
        let row = self.event_row(e, seq, ctor, who, vec![event])?;
        changes.insert(self.audit, row);
        Ok(())
    }

    fn event_row<R: Runtime>(
        &self,
        e: &Engine<R>,
        seq: i64,
        ctor: &str,
        (role, principal): (&str, &Val),
        event: Vec<(&str, Val)>,
    ) -> Result<Val, Error> {
        let ty = Ty::Data {
            def: self.schema.principal,
            args: Vec::new(),
            ctors: None,
        };
        let principal = print(principal, &Form::of(self.schema, &ty))
            .map_err(|_| malformed("a principal prints"))?;
        let mut fields = vec![
            ("seq", Val::Int(seq)),
            ("at", Val::Instant(e.clock())),
            ("role", Val::text(role)),
            ("principal", Val::text(&principal)),
        ];
        fields.extend(event);
        let def = self.schema.ty(self.schema.relation(self.audit).elem_def);
        let ctor = def
            .ctor_named(ctor)
            .unwrap_or_else(|| panic!("the catalog relation declares {ctor}"));
        Ok(self.row(self.audit, ctor, fields))
    }

    pub(super) fn audit_relation(&self) -> RelId {
        self.audit
    }
}

/// What a call runs, as the `origin` of its audit rows states it.
#[derive(Clone, Debug)]
pub(in super::super) enum CallOrigin {
    /// A query or transaction of `bundle` by ordinal, 0 for a refused bind.
    Registered { bundle: String, query: i32 },
    /// A schema function called directly.
    Function { name: String },
    /// Ad hoc text by its hash.
    AdHoc([u8; 32]),
}

/// A call as its `Call` audit row states it, `failed` its error unless it succeeded.
pub(super) struct CallRow<'c> {
    pub(super) role: &'c str,
    pub(super) caller: &'c Val,
    pub(super) origin: &'c CallOrigin,
    pub(super) args: &'c [u8; 32],
    pub(super) failed: Option<&'c Error>,
}

/// Returns the `Service` principal of role `name`.
pub(super) fn service(schema: &Schema, name: &str) -> Val {
    Val::ctor(
        schema.engine_ctors().service().index(),
        vec![Val::text(name)],
    )
}

/// A row of `catalog::credentials`, read for its role.
struct CredentialRow {
    role: i64,
}

const CREDENTIAL: Fields = &[
    "id",
    "role",
    "salt",
    "iterations",
    "stored_key",
    "server_key",
];
const CREDENTIAL_ROLE: Field = field(CREDENTIAL, "role");

impl Decode for CredentialRow {
    const RELATION: CatRelId = engine_rel::CREDENTIALS;
    const CTORS: &'static [Ctor<CredentialRow>] = &[Ctor::new(k::CREDENTIAL, CREDENTIAL, |r| {
        Ok(CredentialRow {
            role: r.int(CREDENTIAL_ROLE),
        })
    })];
}

/// A row of `catalog::peer_mappings`, read for its role.
struct PeerRow {
    role: String,
}

const PEER: Fields = &["listener", "ordinal", "pattern", "role"];
const PEER_ROLE: Field = field(PEER, "role");

impl Decode for PeerRow {
    const RELATION: CatRelId = engine_rel::PEER_MAPPINGS;
    const CTORS: &'static [Ctor<PeerRow>] = &[Ctor::new(k::PEER_MAPPING, PEER, |r| {
        Ok(PeerRow {
            role: r.text(PEER_ROLE).to_owned(),
        })
    })];
}

/// A row of `catalog::audit`, read for its sequence number.
struct AuditRow {
    seq: i64,
}

const LOGIN: Fields = &["seq", "at", "role", "principal", "source", "outcome"];
const GRANT: Fields = &["seq", "at", "role", "principal", "statement"];
const REGISTER: Fields = &["seq", "at", "role", "principal", "bundle", "version"];
const RETIRE: Fields = &["seq", "at", "role", "principal", "bundle"];
const SCHEMA_OP: Fields = &["seq", "at", "role", "principal", "version"];
const CALL: Fields = &[
    "seq",
    "at",
    "role",
    "principal",
    "origin",
    "args",
    "outcome",
];
const DENIED: Fields = &["seq", "at", "role", "principal", "origin"];
const COMMIT: Fields = &[
    "seq",
    "at",
    "role",
    "principal",
    "position",
    "origin",
    "changes",
];
const TRUNCATE: Fields = &["seq", "at", "role", "principal", "through", "digest"];

const LOGIN_SEQ: Field = field(LOGIN, "seq");
const GRANT_SEQ: Field = field(GRANT, "seq");
const REGISTER_SEQ: Field = field(REGISTER, "seq");
const RETIRE_SEQ: Field = field(RETIRE, "seq");
const SCHEMA_OP_SEQ: Field = field(SCHEMA_OP, "seq");
const CALL_SEQ: Field = field(CALL, "seq");
const DENIED_SEQ: Field = field(DENIED, "seq");
const COMMIT_SEQ: Field = field(COMMIT, "seq");
const TRUNCATE_SEQ: Field = field(TRUNCATE, "seq");

impl Decode for AuditRow {
    const RELATION: CatRelId = engine_rel::AUDIT;
    const CTORS: &'static [Ctor<AuditRow>] = &[
        Ctor::new(k::AUDIT_LOGIN, LOGIN, |r| {
            Ok(AuditRow {
                seq: r.int(LOGIN_SEQ),
            })
        }),
        Ctor::new(k::AUDIT_GRANT, GRANT, |r| {
            Ok(AuditRow {
                seq: r.int(GRANT_SEQ),
            })
        }),
        Ctor::new(k::AUDIT_REGISTER, REGISTER, |r| {
            Ok(AuditRow {
                seq: r.int(REGISTER_SEQ),
            })
        }),
        Ctor::new(k::AUDIT_RETIRE, RETIRE, |r| {
            Ok(AuditRow {
                seq: r.int(RETIRE_SEQ),
            })
        }),
        Ctor::new(k::AUDIT_SCHEMA_OP, SCHEMA_OP, |r| {
            Ok(AuditRow {
                seq: r.int(SCHEMA_OP_SEQ),
            })
        }),
        Ctor::new(k::AUDIT_CALL, CALL, |r| {
            Ok(AuditRow {
                seq: r.int(CALL_SEQ),
            })
        }),
        Ctor::new(k::AUDIT_DENIED, DENIED, |r| {
            Ok(AuditRow {
                seq: r.int(DENIED_SEQ),
            })
        }),
        Ctor::new(k::AUDIT_COMMIT, COMMIT, |r| {
            Ok(AuditRow {
                seq: r.int(COMMIT_SEQ),
            })
        }),
        Ctor::new(k::AUDIT_TRUNCATE, TRUNCATE, |r| {
            Ok(AuditRow {
                seq: r.int(TRUNCATE_SEQ),
            })
        }),
    ];
}

fn malformed(detail: impl Into<String>) -> Error {
    Error::Fatal {
        reason: FatalReason::Malformed {
            detail: detail.into(),
        },
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "outcome_tests.rs"]
mod outcome_tests;
