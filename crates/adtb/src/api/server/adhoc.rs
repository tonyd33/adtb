//! What ad hoc text of a role reads and writes against what the role holds.

use std::collections::BTreeSet;

use adtb_lang::check::Command;
use adtb_lang::evolve::{DeployedGrant, Granted};
use adtb_lang::ids::CatFunctionId;
use adtb_lang::permission::{self, EffectiveGrants, Mode, Nameable, Requirement, Requirements};
use adtb_lang::schema::Schema;
use adtb_lang::types::{FnId, RelId};

use crate::runtime::Runtime;

use super::super::cause::Broken;
use super::super::command::engine_relation;
use super::super::dump::read_by_dump;
use super::super::error::qualified;
use super::super::text::{Issuer, Prepared};
use super::super::{Engine, Error, RejectReason};
use super::session::RoleSession;
use super::tables::{CallOrigin, CallRow, Grant, Tables};
use super::{Capability, RoleAuthority, denied};

/// The catalog relations the printed schema states.
const SCHEMA_RELATIONS: [&str; 15] = [
    "types",
    "constructors",
    "ctor_states",
    "value_states",
    "fields",
    "relations",
    "named_sets",
    "set_members",
    "indexes",
    "index_ctors",
    "constraints",
    "functions",
    "defaults",
    "computed",
    "grants",
];

/// The columns whose values reveal rows of constructor relations, each by module, relation and
/// field.
const REVEALING: [(&str, &str, &str); 6] = [
    ("sys", "field_values", "value"),
    ("sys", "conflicts", "keys"),
    ("sys", "verify_findings", "witness"),
    ("catalog", "plans", "text"),
    ("catalog", "plans", "document"),
    ("catalog", "audit", "changes"),
];

/// What a role held when its text was last checked.
#[derive(Clone, Debug)]
pub(in super::super) struct Holds {
    grants: EffectiveGrants,
    pub(in super::super) monitor: bool,
    pub(in super::super) cancel: bool,
    pub(in super::super) schema_capability: bool,
    /// Whether the role may read every catalog relation the printed schema states.
    pub(in super::super) schema: bool,
    /// Whether the role may read every field of every relation outside the engine's modules.
    data: bool,
    /// The functions the role holds `call fn` on.
    calls: BTreeSet<FnId>,
    pub(super) names: Nameable,
}

fn schema_relations(schema: &Schema) -> Result<Vec<RelId>, Error> {
    SCHEMA_RELATIONS
        .iter()
        .map(|n| engine_relation(schema, "catalog", n))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|b| Error::internal(Broken::Write(b)))
}

/// The one answer to text a role may not run, whatever the text names.
pub(super) fn refused_text(role: &str) -> Error {
    denied(format!("role `{role}` may not run this text"))
}

impl Holds {
    fn of<R: Runtime>(e: &Engine<R>, name: &str, grants: &[Grant]) -> Result<Holds, Error> {
        let deployed: Vec<_> = grants.iter().filter_map(Grant::deployed).collect();
        let held = EffectiveGrants::of(&e.schema, &e.source.grants, &deployed, name);
        let covers = |relations: Vec<RelId>| {
            Requirements::whole(&e.schema, relations)
                .map(|r| r.missing(&held, Mode::Ordinary).is_empty())
                .map_err(no_identity)
        };
        let catalog = schema_relations(&e.schema)?;
        let user = (0..e.schema.relations.len() as u32)
            .map(RelId)
            .filter(|r| !e.schema.module(e.schema.relation(*r).module).engine)
            .collect();
        let schema = covers(catalog)?;
        let data = covers(user)?;
        let has = |c| grants.iter().any(|g| g.capability(c, name));
        let calls = granted_calls(e, name, grants);
        let names = Nameable::of(&e.schema, &held, calls.clone());
        Ok(Holds {
            monitor: has(Capability::Monitor),
            cancel: has(Capability::Cancel),
            schema_capability: has(Capability::Schema),
            schema,
            data,
            calls,
            names,
            grants: held,
        })
    }

    /// Returns what `needs`, of text over `schema`, asks beyond what the role holds. `monitor`
    /// stands for a read of a `sys::` relation or `catalog::plans`, and a column revealing rows
    /// needs a read of every row.
    fn lacking(&self, schema: &Schema, needs: &Requirements) -> Vec<Requirement> {
        let revealing = revealing(schema);
        let mut out: Vec<_> = needs
            .missing(&self.grants, Mode::Ordinary)
            .into_iter()
            .filter(|r| !(self.monitor && monitored(schema, r)))
            .collect();
        if !self.data {
            out.extend(
                needs
                    .iter()
                    .filter(
                        |r| matches!(r, Requirement::ReadField(_, f) if revealing.contains(&f.raw())),
                    )
                    .cloned(),
            );
        }
        out
    }

    /// Returns whether the role may read what the constraint at position `at` of `schema` reads.
    pub(in super::super) fn reads_constraint<R: Runtime>(&self, e: &Engine<R>, at: usize) -> bool {
        let normal = e.db.normal();
        normal.constraints.get(at).is_some_and(|t| {
            permission::normal(&e.schema, normal, &normal.terms, *t, &[])
                .is_ok_and(|needs| self.lacking(&e.schema, &needs).is_empty())
        })
    }

    /// Returns `err` with what the role may not read taken out of a violation's report.
    pub(super) fn redact<R: Runtime>(&self, e: &Engine<R>, err: Error) -> Error {
        let Error::Rejected { reason } = err else {
            return err;
        };
        let reason = match reason {
            RejectReason::Violated {
                constraint,
                report,
                redacted,
            } => {
                let visible = e
                    .schema
                    .constraints
                    .iter()
                    .position(|c| {
                        constraint
                            .strip_prefix(e.schema.module(c.module).name.as_str())
                            .and_then(|n| n.strip_prefix("::"))
                            == Some(c.name.as_str())
                    })
                    .is_some_and(|at| self.reads_constraint(e, at));
                if visible {
                    RejectReason::Violated {
                        constraint,
                        report,
                        redacted,
                    }
                } else {
                    let left = i64::try_from(report.len()).unwrap_or(i64::MAX);
                    RejectReason::Violated {
                        constraint,
                        report: Vec::new(),
                        redacted: redacted.saturating_add(left),
                    }
                }
            }
            RejectReason::KeyViolation { relation, key } => {
                let held = (0..e.schema.relations.len() as u32)
                    .map(RelId)
                    .find(|r| qualified(&e.schema, *r) == relation)
                    .and_then(|r| Requirements::whole(&e.schema, [r]).ok())
                    .is_some_and(|needs| self.lacking(&e.schema, &needs).is_empty());
                RejectReason::KeyViolation {
                    relation,
                    key: if held { key } else { String::new() },
                }
            }
            other => other,
        };
        Error::Rejected { reason }
    }
}

/// Returns the functions `name` holds a grant to call directly.
fn granted_calls<R: Runtime>(e: &Engine<R>, name: &str, grants: &[Grant]) -> BTreeSet<FnId> {
    grants
        .iter()
        .filter(|g| g.grantee == name && g.privilege == "call" && !g.dormant && g.kind == "Log")
        .filter_map(
            |g| match Granted::read_target(&e.schema, "call", g.target.as_deref()?) {
                Some(Granted::CallFn(id)) => {
                    e.schema.ids().function_of(CatFunctionId::from_raw(id))
                }
                _ => None,
            },
        )
        .collect()
}

fn no_identity(e: permission::Error) -> Error {
    if e == permission::Error::TooLarge {
        return Error::invalid("the text asks more of the permission check than it allows");
    }
    Error::internal(Broken::NoIdentity(e))
}

/// Returns whether `deployed` gives `role` the `monitor` capability.
pub(in super::super) fn holds_monitor(deployed: &[DeployedGrant], role: &str) -> bool {
    deployed.iter().any(|g| {
        g.grantee == role
            && g.privilege == Capability::Monitor.name()
            && g.target.is_none()
            && !g.dormant
    })
}

/// Returns whether `r` reads a `sys::` relation or `catalog::plans`.
pub(in super::super) fn monitored(schema: &Schema, r: &Requirement) -> bool {
    if !matches!(
        r,
        Requirement::ReadConstructor(_) | Requirement::ReadField(..)
    ) {
        return false;
    }
    let Some(rel) = r.relation().and_then(|id| schema.ids().relation_of(id)) else {
        return false;
    };
    let plans = engine_relation(schema, "catalog", "plans").ok();
    Some(rel) == plans || schema.module_named("sys") == Some(schema.relation(rel).module)
}

/// Returns the ids of the fields of [`REVEALING`].
fn revealing(schema: &Schema) -> BTreeSet<u64> {
    let mut out = BTreeSet::new();
    for (module, relation, field) in REVEALING {
        let Ok(rel) = engine_relation(schema, module, relation) else {
            continue;
        };
        let td = schema.ty(schema.relation(rel).elem_def);
        out.extend(
            schema
                .ids()
                .type_field_ids(td.id())
                .filter(|(f, _)| td.field_def(*f).name == *field)
                .map(|(_, id)| id.raw()),
        );
    }
    out
}

impl<R: Runtime> RoleAuthority<R> {
    /// Returns what `session`'s role holds now.
    pub(super) fn holds(&self, session: &RoleSession<R>) -> Result<Holds, Error> {
        self.access(false, |e, tx, id, _| {
            let t = Tables::new(&e.schema, id);
            self.checked(session, &t, e, tx)?;
            Holds::of(e, &session.name, &t.grants(e, tx)?)
        })
    }

    /// Refuse text `p` of `session` unless the role now holds all it reads and writes, with
    /// `Denied` recorded in the audit, and name what it lacks only to a role that may read the
    /// schema. Returns what the role holds.
    pub(super) fn admit_prepared(
        &self,
        session: &RoleSession<R>,
        e: &Engine<R>,
        p: &Prepared,
        caller: &crate::exec::Val,
        issuer: &Issuer,
        args: &[u8; 32],
    ) -> Result<Holds, Error> {
        let holds = self.holds(session)?;
        let params: Vec<_> = p.params().iter().map(|(v, _)| *v).collect();
        let normal = e.db.normal();
        let origin = CallOrigin::AdHoc(issuer.text);
        let needs = |root| {
            permission::normal(p.schema(), normal, p.terms(), root, &params).map_err(|err| {
                let err = no_identity(err);
                e.audit_call(&CallRow {
                    role: &session.name,
                    caller,
                    origin: &origin,
                    args,
                    failed: Some(&err),
                })
                .map_or_else(|audit| audit, |()| err)
            })
        };
        let mut lacking = Vec::new();
        let roots = p
            .items()
            .iter()
            .copied()
            .chain(p.commands().iter().flat_map(|c| c.command.terms()));
        for root in roots {
            lacking.extend(holds.lacking(p.schema(), &needs(root)?));
        }
        if p.commands()
            .iter()
            .any(|c| matches!(c.command, Command::Dump { .. }))
        {
            let mut read = read_by_dump(p.schema());
            read.extend(schema_relations(p.schema())?);
            let dumped = Requirements::whole(p.schema(), read);
            lacking.extend(holds.lacking(p.schema(), &dumped.map_err(no_identity)?));
        }
        let uncalled = p.functions().iter().find(|f| {
            let def = &p.schema().functions[f.0 as usize];
            !p.schema().module(def.module).engine && !holds.calls.contains(f)
        });
        let unseen = p
            .commands()
            .iter()
            .any(|c| matches!(c.command, Command::PrintSchema))
            && !holds.schema;
        let Some(detail) = lacking
            .first()
            .map(|r| r.text(p.schema()))
            .or(uncalled.map(|f| {
                let name = &p.schema().functions[f.0 as usize].name;
                (format!("calls `{name}`"), "call")
            }))
            .or(unseen.then(|| ("reads the schema".to_owned(), "read")))
        else {
            return Ok(holds);
        };
        if let Some(quarantined) = e.refusal() {
            return Err(quarantined);
        }
        e.audit_denied(&session.name, caller, &origin)?;
        Err(if holds.schema {
            denied(format!(
                "role `{}` {} and lacks {}",
                session.name, detail.0, detail.1
            ))
        } else {
            refused_text(&session.name)
        })
    }
}
