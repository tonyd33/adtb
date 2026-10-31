use std::collections::BTreeMap;
use std::sync::Arc;

use adtb_lang::evolve::GrantRow;
use adtb_lang::schema::{Schema, TypeDef, ValueDef};
use adtb_lang::textual::{Form, parse, print};
use adtb_lang::types::{RelId, Ty};

use crate::catalog::layout;
use crate::exec::Val;
use crate::runtime::Runtime;
use crate::write::Txn;

use super::super::{Engine, Error, FatalReason, RejectReason, TxnId};
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
        self.kind == "Deployment"
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
            kind: "Deployment".to_owned(),
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
            g.kind == "Deployment"
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
                let id = self.next_id(e, tx, self.grants)?;
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
        privilege: &str,
        target: Option<&str>,
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

    /// Record that `role`, running for `caller`, was refused ad hoc text of hash `text`.
    pub(super) fn audit_denied<R: Runtime>(
        &self,
        e: &Engine<R>,
        changes: &mut Changes,
        role: &str,
        caller: &Val,
        text: &[u8; 32],
    ) -> Result<(), Error> {
        let event = ("origin", self.adhoc_origin(text)?);
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

    /// Returns the `Call` row numbered `seq` of ad hoc text `call`.
    pub(super) fn call_row<R: Runtime>(
        &self,
        e: &Engine<R>,
        seq: i64,
        call: &CallRow<'_>,
    ) -> Result<Val, Error> {
        let fields = vec![
            ("origin", self.adhoc_origin(call.text)?),
            ("args", Val::Bytes(Arc::from(call.args.as_slice()))),
            ("outcome", self.outcome(call.failed)?),
        ];
        self.event_row(e, seq, "Call", (call.role, call.caller), fields)
    }

    fn adhoc_origin(&self, text: &[u8; 32]) -> Result<Val, Error> {
        let adhoc = self
            .prelude_type("Origin")?
            .ctor_index("AdHoc")
            .ok_or_else(|| malformed("`Origin` declares `AdHoc`"))?;
        Ok(Val::ctor(
            adhoc,
            vec![Val::Bytes(Arc::from(text.as_slice()))],
        ))
    }

    fn outcome(&self, failed: Option<&Error>) -> Result<Val, Error> {
        let def = self.prelude_type("Outcome")?;
        let ctor = |name: &str| {
            def.ctor_index(name)
                .ok_or_else(|| malformed(format!("`Outcome` declares `{name}`")))
        };
        Ok(match failed {
            None => Val::ctor(ctor("Succeeded")?, Vec::new()),
            Some(error) => Val::ctor(ctor("Failed")?, vec![self.error_value(error)?]),
        })
    }

    /// Returns `error` as a value of the prelude sum, or as `Internal` naming it when its text
    /// does not read back.
    fn error_value(&self, error: &Error) -> Result<Val, Error> {
        let prelude = self
            .schema
            .module_named("prelude")
            .ok_or_else(|| malformed("the schema has module prelude"))?;
        let ty = Ty::Data {
            def: self.schema.module(prelude).types["Error"],
            args: Vec::new(),
            ctors: None,
        };
        let form = Form::of(self.schema, &ty);
        let text = error.value().to_string();
        parse::<Val>(&text, &form).or_else(|_| {
            let internal = Error::rejected(RejectReason::Internal { what: text.clone() });
            parse::<Val>(&internal.value().to_string(), &form)
                .map_err(|_| malformed(format!("the error `{text}` reads as a value")))
        })
    }

    fn prelude_type(&self, name: &str) -> Result<&'s TypeDef, Error> {
        let prelude = self
            .schema
            .module_named("prelude")
            .ok_or_else(|| malformed("the schema has module prelude"))?;
        let id = self
            .schema
            .module(prelude)
            .types
            .get(name)
            .ok_or_else(|| malformed(format!("module prelude declares `{name}`")))?;
        Ok(self.schema.ty(*id))
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
        Ok(self.row(self.audit, ctor, fields))
    }

    pub(super) fn audit_relation(&self) -> RelId {
        self.audit
    }
}

/// An ad hoc call as its `Call` audit row states it, `failed` its error unless it succeeded.
pub(super) struct CallRow<'c> {
    pub(super) role: &'c str,
    pub(super) caller: &'c Val,
    pub(super) text: &'c [u8; 32],
    pub(super) args: &'c [u8; 32],
    pub(super) failed: Option<&'c Error>,
}

/// Returns the `Service` principal of role `name`.
pub(super) fn service(schema: &Schema, name: &str) -> Val {
    let index = schema
        .ty(schema.principal)
        .ctor_index("Service")
        .expect("the Principal type declares Service");
    Val::ctor(index, vec![Val::text(name)])
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
