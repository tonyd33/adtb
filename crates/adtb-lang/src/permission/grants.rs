use crate::evolve::{DeployedGrant, GrantTarget, Granted, LogGrant};

use super::*;

impl EffectiveGrants {
    /// Returns what `role` holds among `grants`, a log over `schema`, and `deployed`, each
    /// relation grant resolved to the constructors it covers in `schema`. A dormant grant allows
    /// nothing, and a deployed target `schema` cannot read covers nothing.
    pub fn of(
        schema: &Schema,
        grants: &[LogGrant],
        deployed: &[DeployedGrant],
        role: &str,
    ) -> EffectiveGrants {
        let mut relations = Vec::new();
        for g in grants.iter().filter(|g| g.role == role && !g.dormant) {
            rights(schema, &g.privilege, g.grant_option, &mut relations);
        }
        let deployed = || deployed.iter().filter(|g| g.grantee == role && !g.dormant);
        for g in deployed().filter(|g| g.privilege != "call") {
            let held = g
                .target
                .as_deref()
                .and_then(|t| Granted::read_target(schema, &g.privilege, t));
            if let Some(privilege) = held {
                rights(schema, &privilege, g.grant_option, &mut relations);
            }
        }
        EffectiveGrants {
            relations,
            read_files: deployed().any(|g| g.privilege == "read_files" && g.target.is_none()),
        }
    }
}

/// Add to `out` each right `privilege` gives over `schema`.
fn rights(schema: &Schema, privilege: &Granted, grant_option: bool, out: &mut Vec<EffectiveGrant>) {
    let mut push = |right| {
        out.push(EffectiveGrant {
            right,
            grant_option,
        });
    };
    match privilege {
        Granted::Read(t, fs) | Granted::Update(t, fs) => {
            let fields: BTreeSet<FieldId> = fs.ids.iter().copied().map(FieldId).collect();
            let read = matches!(privilege, Granted::Read(..));
            for target in targets(schema, t) {
                let fields = fields.clone();
                push(if read {
                    Right::Read { target, fields }
                } else {
                    Right::Update { target, fields }
                });
            }
        }
        Granted::Insert(t) => targets(schema, t)
            .into_iter()
            .for_each(|t| push(Right::Insert(t))),
        Granted::Delete(t) => targets(schema, t)
            .into_iter()
            .for_each(|t| push(Right::Delete(t))),
        Granted::Retag(from, to, except) => {
            let to = targets(schema, to);
            for f in targets(schema, from) {
                for t in &to {
                    let pair = (f.constructor.0, t.constructor.0);
                    if f.constructor != t.constructor && !except.contains(&pair) {
                        push(Right::Retag {
                            relation: f.relation,
                            from: f.constructor,
                            to: t.constructor,
                        });
                    }
                }
            }
        }
        Granted::CallFn(_) => {}
    }
}

/// Returns each constructor of `schema` that `t` covers.
fn targets(schema: &Schema, t: &GrantTarget) -> Vec<Target> {
    t.ctors(schema)
        .into_iter()
        .map(|id| Target {
            relation: RelationId(t.relation),
            constructor: ConstructorId(id),
        })
        .collect()
}

impl Requirement {
    /// Returns what the requirement does, as `reads identities.EmailPass (hash)`, and the
    /// privilege it needs, as `read`.
    pub fn text(&self, schema: &Schema) -> (String, &'static str) {
        let at = |t: &Target| {
            let rel = schema.ids.relation(t.relation.0);
            let r = rel.map_or("?", |r| schema.relation(r).name.as_str());
            let c = schema
                .ids
                .ctors
                .iter()
                .find(|(_, id)| **id == t.constructor.0)
                .and_then(|(&(def, c), _)| schema.ty(def).ctors().get(c as usize))
                .map_or("?", |c| c.name.as_str());
            format!("{r}.{c}")
        };
        let field = |f: &FieldId| {
            schema
                .ids
                .fields
                .iter()
                .find(|(_, id)| **id == f.0)
                .map_or("?", |((_, _, name), _)| name.as_str())
        };
        match self {
            Requirement::ReadConstructor(t) => (format!("reads {}", at(t)), "read"),
            Requirement::ReadField(t, f) => (format!("reads {} ({})", at(t), field(f)), "read"),
            Requirement::Insert(t) => (format!("inserts {}", at(t)), "insert"),
            Requirement::Delete(t) => (format!("deletes {}", at(t)), "delete"),
            Requirement::UpdateField(t, f) => {
                (format!("updates {} ({})", at(t), field(f)), "update")
            }
            Requirement::Retag { relation, from, to } => {
                let t = |c| Target {
                    relation: *relation,
                    constructor: c,
                };
                (
                    format!("retags {} to {}", at(&t(*from)), at(&t(*to))),
                    "retag",
                )
            }
            Requirement::ReadFiles => ("reads files".to_owned(), "read_files"),
        }
    }
}
