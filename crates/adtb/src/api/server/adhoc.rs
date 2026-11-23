//! What ad hoc text of a role reads and writes against what the role holds.

use std::collections::BTreeSet;

use adtb_lang::check::Command;
use adtb_lang::evolve::{DeployedGrant, Granted, Privilege};
use adtb_lang::ids::{CatFieldId, CatFunctionId, EngineRel, engine_field, engine_rel};
use adtb_lang::permission::{self, EffectiveGrants, Mode, Nameable, Requirement, Requirements};
use adtb_lang::schema::Schema;
use adtb_lang::types::{DenialIdx, FnId, RelId};

use crate::catalog::codec::GrantKind;
use crate::catalog::engine_relation;
use crate::runtime::Runtime;
use crate::write;

use super::super::cause::{Broken, past_limits};
use super::super::dump::read_by_dump;
use super::super::text::{Issuer, Prepared};
use super::super::{Engine, Error, RejectReason};
use super::session::RoleSession;
use super::tables::{CallOrigin, CallRow, Grant, Tables, deployed};
use super::{Capability, RoleAuthority, denied};

/// The catalog relations the printed schema states.
const SCHEMA_RELATIONS: [EngineRel; 15] = [
    engine_rel::TYPES,
    engine_rel::CONSTRUCTORS,
    engine_rel::CTOR_STATES,
    engine_rel::VALUE_STATES,
    engine_rel::FIELDS,
    engine_rel::RELATIONS,
    engine_rel::NAMED_SETS,
    engine_rel::SET_MEMBERS,
    engine_rel::INDEXES,
    engine_rel::INDEX_CTORS,
    engine_rel::CONSTRAINTS,
    engine_rel::FUNCTIONS,
    engine_rel::DEFAULTS,
    engine_rel::COMPUTED,
    engine_rel::GRANTS,
];

/// The columns whose values reveal rows of constructor relations.
const REVEALING: [CatFieldId; 6] = [
    engine_field::FIELD_VALUES_VALUE,
    engine_field::CONFLICTS_KEYS,
    engine_field::VERIFY_FINDINGS_WITNESS,
    engine_field::PLANS_TEXT,
    engine_field::PLANS_DOCUMENT,
    engine_field::AUDIT_CHANGES,
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

fn schema_relations(schema: &Schema) -> Vec<RelId> {
    SCHEMA_RELATIONS
        .iter()
        .map(|&id| engine_relation(schema, id))
        .collect()
}

/// The one answer to text a role may not run, whatever the text names.
pub(super) fn refused_text(role: &str) -> Error {
    denied(format!("role `{role}` may not run this text"))
}

impl Holds {
    fn of<R: Runtime>(e: &Engine<R>, name: &str, grants: &[Grant]) -> Result<Holds, Error> {
        let deployed = deployed(grants);
        let held = EffectiveGrants::of(e.schema(), &e.source.grants, &deployed, name);
        let covers = |relations: Vec<RelId>| {
            Requirements::whole(e.schema(), relations)
                .map(|r| r.missing(&held, Mode::Ordinary).is_empty())
                .map_err(no_identity)
        };
        let catalog = schema_relations(e.schema());
        let user = (0..e.schema().relations.len() as u32)
            .map(RelId)
            .filter(|r| !e.schema().module(e.schema().relation(*r).module).engine)
            .collect();
        let schema = covers(catalog)?;
        let data = covers(user)?;
        let has = |c| grants.iter().any(|g| g.capability(c, name));
        let calls = granted_calls(e, name, grants);
        let names = Nameable::of(e.schema(), &held, calls.clone());
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
        let mut out: Vec<_> = needs
            .missing(&self.grants, Mode::Ordinary)
            .into_iter()
            .filter(|r| !(self.monitor && monitored(schema, r)))
            .collect();
        if !self.data {
            out.extend(
                needs
                    .iter()
                    .filter(|r| matches!(r, Requirement::ReadField(_, f) if REVEALING.contains(f)))
                    .cloned(),
            );
        }
        out
    }

    /// Returns whether the role may read what constraint or check `d` reads.
    ///
    /// Fail with [`write::Broken::NoDenial`] when the engine's normal form holds no term of `d`.
    pub(in super::super) fn reads_denial<R: Runtime>(
        &self,
        e: &Engine<R>,
        d: DenialIdx,
    ) -> Result<bool, write::Broken> {
        let normal = e.db.normal();
        let (t, module, name) = match d {
            DenialIdx::Constraint(c) => {
                let def = e.schema().constraint(c);
                (normal.constraint(c), def.module, &def.name)
            }
            DenialIdx::Check(c) => {
                let def = e.schema().check(c);
                (normal.check(c), def.module, &def.name)
            }
        };
        let Some(t) = t else {
            return Err(write::Broken::NoDenial(format!(
                "{}::{name}",
                e.schema().module(module).name
            )));
        };
        Ok(
            permission::normal(e.schema(), normal, normal.terms(), t, &[])
                .is_ok_and(|needs| self.lacking(e.schema(), &needs).is_empty()),
        )
    }

    /// Returns `err` with what the role may not read taken out of a violation's report, the
    /// statement refused having violated `violated`.
    pub(super) fn redact<R: Runtime>(
        &self,
        e: &Engine<R>,
        err: Error,
        violated: Option<write::Violated>,
    ) -> Error {
        let Error::Rejected { reason } = err else {
            return err;
        };
        let reason = match reason {
            RejectReason::Violated {
                constraint,
                report,
                redacted,
            } => {
                let visible = match violated {
                    Some(write::Violated::Other(write::Violator::Denial(d))) => {
                        match self.reads_denial(e, d) {
                            Ok(visible) => visible,
                            Err(b) => return Error::internal(Broken::Write(b)),
                        }
                    }
                    _ => false,
                };
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
                let held = match violated {
                    Some(write::Violated::Key(r)) => Requirements::whole(e.schema(), [r])
                        .is_ok_and(|needs| self.lacking(e.schema(), &needs).is_empty()),
                    _ => false,
                };
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
        .filter(|g| {
            g.grantee == name
                && g.privilege == Privilege::Call
                && !g.dormant
                && g.kind == GrantKind::Log
        })
        .filter_map(|g| {
            match Granted::read_target(e.schema(), Privilege::Call, g.target.as_deref()?) {
                Some(Granted::CallFn(id)) => {
                    e.schema().ids().function_of(CatFunctionId::from_raw(id))
                }
                _ => None,
            }
        })
        .collect()
}

fn no_identity(e: permission::Error) -> Error {
    Error::unanalysed(e, Broken::NoIdentity)
}

/// Returns whether `deployed` gives `role` the `monitor` capability.
pub(in super::super) fn holds_monitor(deployed: &[DeployedGrant], role: &str) -> bool {
    deployed.iter().any(|g| {
        g.grantee == role
            && g.privilege == Privilege::Capability(Capability::Monitor)
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
    rel == engine_relation(schema, engine_rel::PLANS)
        || schema
            .relation(engine_relation(schema, engine_rel::SESSIONS))
            .module
            == schema.relation(rel).module
}

impl<R: Runtime> RoleAuthority<R> {
    /// Returns what `session`'s role holds now.
    pub(super) fn holds(&self, session: &RoleSession<R>) -> Result<Holds, Error> {
        self.access(false, |e, tx, id, _| {
            let t = Tables::new(e.schema(), id);
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
                if !holds.schema && past_limits(&err) {
                    if let Some(quarantined) = e.refusal() {
                        return quarantined;
                    }
                    return e
                        .audit_denied(&session.name, caller, &origin)
                        .map_or_else(|audit| audit, |()| refused_text(&session.name));
                }
                let err = no_identity(err);
                e.audit_call(
                    &CallRow {
                        role: &session.name,
                        caller,
                        origin: &origin,
                        args,
                        failed: Some(&err),
                    },
                    None,
                )
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
            read.extend(schema_relations(p.schema()));
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
