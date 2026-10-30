use std::collections::BTreeMap;
use std::sync::Arc;

use adtb_lang::evolve::GrantRow;
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::textual::{Form, print};
use adtb_lang::types::{RelId, Ty};

use crate::catalog::layout;
use crate::exec::Val;
use crate::runtime::Runtime;
use crate::write::Txn;

use super::super::{Engine, Error, FatalReason, OWNER_ROLE, TxnId};
use super::store::Changes;
use super::{Capability, denied};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RoleId(pub(super) i64);

pub(super) struct Role {
    pub(super) id: RoleId,
    pub(super) name: String,
    pub(super) created: i64,
}

pub(super) struct Grant {
    pub(super) id: i64,
    pub(super) grantee: String,
    pub(super) grantor: String,
    pub(super) grant_option: bool,
    pub(super) kind: String,
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

    pub(super) fn capability(&self, capability: Capability, name: &str) -> bool {
        self.kind == "Deployment"
            && self.privilege == capability.name()
            && self.target.is_none()
            && self.grantee == name
            && !self.dormant
    }
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
    pub(super) fn new(schema: &'s Schema, call: TxnId) -> Self {
        let module = schema
            .module_named("catalog")
            .expect("the engine declares catalog");
        let relation = |name: &str| match schema.module(module).values.get(name) {
            Some(ValueDef::Relation(r)) => *r,
            _ => panic!("the catalog declares relation {name}"),
        };
        Tables {
            schema,
            call,
            roles: relation("roles"),
            grants: relation("grants"),
            credentials: relation("credentials"),
            peers: relation("peer_mappings"),
            audit: relation("audit"),
        }
    }

    pub(super) fn read<R: Runtime>(
        &self,
        e: &Engine<R>,
        tx: &Txn,
        r: RelId,
    ) -> Result<Vec<Val>, Error> {
        e.db.relation_rows(tx, r)
            .map_err(|error| e.write_error(self.call, error))
    }

    fn field<'v>(&self, relation: RelId, row: &'v Val, field: &str) -> Result<&'v Val, Error> {
        let def = self.schema.ty(self.schema.relation(relation).elem_def);
        let Val::Ctor(c, values) = row else {
            return Err(malformed("a catalog row is a constructor"));
        };
        if def.ctors().get(*c as usize).is_none() {
            return Err(malformed("a catalog row has its declared constructor"));
        }
        let fields = layout::row(self.schema, relation, *c);
        if fields.len() != values.len() {
            return Err(malformed("a catalog row has its declared fields"));
        }
        let index = fields
            .iter()
            .position(|(name, _)| name == field)
            .ok_or_else(|| malformed(format!("a catalog row has field `{field}`")))?;
        Ok(&values[index])
    }

    fn text(&self, relation: RelId, row: &Val, field: &str) -> Result<String, Error> {
        match self.field(relation, row, field)? {
            Val::Text(text) => Ok(text.to_string()),
            _ => Err(malformed(format!("catalog field `{field}` is Text"))),
        }
    }

    pub(super) fn integer(&self, relation: RelId, row: &Val, field: &str) -> Result<i64, Error> {
        match self.field(relation, row, field)? {
            Val::Int(value) => Ok(*value),
            _ => Err(malformed(format!(
                "catalog field `{field}` is an integer key"
            ))),
        }
    }

    fn flag(&self, relation: RelId, row: &Val, field: &str) -> Result<bool, Error> {
        self.field(relation, row, field)?
            .truth()
            .map_err(|_| malformed(format!("catalog field `{field}` is Bool")))
    }

    fn row(&self, relation: RelId, ctor: &str, fields: Vec<(&str, Val)>) -> Val {
        let def = self.schema.ty(self.schema.relation(relation).elem_def);
        let index = def
            .ctor_index(ctor)
            .unwrap_or_else(|| panic!("the catalog relation declares {ctor}"));
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
        self.read(e, tx, self.roles)?
            .iter()
            .map(|row| {
                let created = match self.field(self.roles, row, "created")? {
                    Val::Instant(at) => *at,
                    _ => return Err(malformed("a role's creation time is Instant")),
                };
                Ok(Role {
                    id: RoleId(self.integer(self.roles, row, "id")?),
                    name: self.text(self.roles, row, "name")?,
                    created,
                })
            })
            .collect()
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

    pub(super) fn role_value(&self, role: &Role) -> Val {
        self.row(
            self.roles,
            "Role",
            vec![
                ("id", Val::Int(role.id.0)),
                ("name", Val::text(&role.name)),
                ("created", Val::Instant(role.created)),
            ],
        )
    }

    pub(super) fn grants<R: Runtime>(&self, e: &Engine<R>, tx: &Txn) -> Result<Vec<Grant>, Error> {
        self.read(e, tx, self.grants)?
            .into_iter()
            .map(|row| {
                let target = self.optional_text(self.field(self.grants, &row, "target")?)?;
                let module = self
                    .schema
                    .module_named("catalog")
                    .expect("the catalog exists");
                let def = self
                    .schema
                    .ty(self.schema.module(module).types["GrantKind"]);
                let kind = match self.field(self.grants, &row, "kind")? {
                    Val::Ctor(c, fields) if fields.is_empty() => def.ctors().get(*c as usize),
                    _ => None,
                }
                .ok_or_else(|| malformed("a grant kind is Log or Deployment"))?
                .name
                .clone();
                Ok(Grant {
                    id: self.integer(self.grants, &row, "id")?,
                    grantee: self.text(self.grants, &row, "grantee")?,
                    grantor: self.text(self.grants, &row, "grantor")?,
                    grant_option: self.flag(self.grants, &row, "grant_option")?,
                    dormant: self.flag(self.grants, &row, "dormant")?,
                    privilege: self.text(self.grants, &row, "privilege")?,
                    target,
                    kind,
                    value: row,
                })
            })
            .collect()
    }

    fn optional_text(&self, value: &Val) -> Result<Option<String>, Error> {
        let option = self.schema.ty(self.schema.prelude_type("Option"));
        let Val::Ctor(ctor, fields) = value else {
            return Err(malformed("a grant target is Option Text"));
        };
        match (
            option.ctors().get(*ctor as usize).map(|c| c.name.as_str()),
            fields.as_ref(),
        ) {
            (Some("None"), []) => Ok(None),
            (Some("Some"), [Val::Text(text)]) => Ok(Some(text.to_string())),
            _ => Err(malformed("a grant target is Option Text")),
        }
    }

    pub(super) fn grant_value(
        &self,
        id: i64,
        role: &str,
        capability: Capability,
        grantor: &str,
        grant_option: bool,
    ) -> Val {
        let module = self
            .schema
            .module_named("catalog")
            .expect("the catalog exists");
        let kind = self
            .schema
            .ty(self.schema.module(module).types["GrantKind"])
            .ctor_index("Deployment")
            .expect("GrantKind declares Deployment");
        self.row(
            self.grants,
            "RoleGrant",
            vec![
                ("id", Val::Int(id)),
                ("kind", Val::ctor(kind, Vec::new())),
                ("privilege", Val::text(capability.name())),
                ("target", Val::none()),
                ("grantee", Val::text(role)),
                ("grantor", Val::text(grantor)),
                ("grant_option", Val::bool(grant_option)),
                ("dormant", Val::bool(false)),
            ],
        )
    }

    pub(super) fn next_id<R: Runtime>(
        &self,
        e: &Engine<R>,
        tx: &Txn,
        relation: RelId,
    ) -> Result<i64, Error> {
        let field = if relation == self.audit { "seq" } else { "id" };
        let highest = self
            .read(e, tx, relation)?
            .iter()
            .map(|row| self.integer(relation, row, field))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .max()
            .unwrap_or(0);
        let floor = highest
            .checked_add(1)
            .ok_or_else(|| Error::invalid("the deployment identity range is exhausted"))?;
        e.db.deployment_id(relation, floor)
            .map_err(|error| e.write_error(self.call, error))
    }

    pub(super) fn remove_peer_mappings<R: Runtime>(
        &self,
        e: &Engine<R>,
        tx: &Txn,
        changes: &mut Changes,
        name: &str,
    ) -> Result<(), Error> {
        for row in self.read(e, tx, self.peers)? {
            if self.text(self.peers, &row, "role")? == name {
                changes.delete(self.peers, row);
            }
        }
        Ok(())
    }

    pub(super) fn audit<R: Runtime>(
        &self,
        e: &Engine<R>,
        tx: &Txn,
        changes: &mut Changes,
        statement: String,
    ) -> Result<(), Error> {
        let principal = e.principal(None)?;
        let event = ("statement", Val::text(&statement));
        self.audit_row(e, tx, changes, "Grant", (OWNER_ROLE, &principal), event)
    }

    /// Record that `role`, running for `caller`, was refused ad hoc text of hash `text`.
    pub(super) fn audit_denied<R: Runtime>(
        &self,
        e: &Engine<R>,
        tx: &Txn,
        changes: &mut Changes,
        role: &str,
        caller: &Val,
        text: &[u8; 32],
    ) -> Result<(), Error> {
        let prelude = self
            .schema
            .module_named("prelude")
            .ok_or_else(|| malformed("the schema has module prelude"))?;
        let origin = self.schema.module(prelude).types["Origin"];
        let adhoc = self
            .schema
            .ty(origin)
            .ctor_index("AdHoc")
            .ok_or_else(|| malformed("`Origin` declares `AdHoc`"))?;
        let event = (
            "origin",
            Val::ctor(adhoc, vec![Val::Bytes(Arc::from(text.as_slice()))]),
        );
        self.audit_row(e, tx, changes, "Denied", (role, caller), event)
    }

    fn audit_row<R: Runtime>(
        &self,
        e: &Engine<R>,
        tx: &Txn,
        changes: &mut Changes,
        ctor: &str,
        (role, principal): (&str, &Val),
        event: (&str, Val),
    ) -> Result<(), Error> {
        let ty = Ty::Data {
            def: self.schema.principal,
            args: Vec::new(),
            ctors: None,
        };
        let principal = print(principal, &Form::of(self.schema, &ty))
            .map_err(|_| malformed("a principal prints"))?;
        let id = self.next_id(e, tx, self.audit)?;
        changes.insert(
            self.audit,
            self.row(
                self.audit,
                ctor,
                vec![
                    ("seq", Val::Int(id)),
                    ("at", Val::Instant(e.clock())),
                    ("role", Val::text(role)),
                    ("principal", Val::text(&principal)),
                    event,
                ],
            ),
        );
        Ok(())
    }
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
