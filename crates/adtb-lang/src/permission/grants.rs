use crate::evolve::{GrantSel, GrantTarget, Granted, LogGrant};

use super::*;

impl EffectiveGrants {
    /// Returns the relation grants `role` holds among `grants`, a log over `schema`, each
    /// resolved to the constructors it covers in `schema`. A dormant grant allows nothing.
    pub fn of(schema: &Schema, grants: &[LogGrant], role: &str) -> EffectiveGrants {
        let mut relations = Vec::new();
        for g in grants.iter().filter(|g| g.role == role && !g.dormant) {
            let mut push = |right| {
                relations.push(EffectiveGrant {
                    right,
                    grant_option: g.grant_option,
                });
            };
            match &g.privilege {
                Granted::Read(t, fs) | Granted::Update(t, fs) => {
                    let fields: BTreeSet<FieldId> = fs.ids.iter().copied().map(FieldId).collect();
                    let read = matches!(g.privilege, Granted::Read(..));
                    for target in targets(schema, t) {
                        let fields = fields.clone();
                        push(if read {
                            Right::Read { target, fields }
                        } else {
                            Right::Update { target, fields }
                        });
                    }
                }
                Granted::Insert(t) => targets(schema, t).into_iter().for_each(|t| {
                    push(Right::Insert(t));
                }),
                Granted::Delete(t) => targets(schema, t).into_iter().for_each(|t| {
                    push(Right::Delete(t));
                }),
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
        EffectiveGrants {
            relations,
            read_files: false,
        }
    }
}

/// Returns each constructor of `schema` that `t` covers.
fn targets(schema: &Schema, t: &GrantTarget) -> Vec<Target> {
    let Some(rel) = relation(schema, t.relation) else {
        return Vec::new();
    };
    let r = schema.relation(rel);
    schema
        .ids
        .ctors
        .iter()
        .filter(|((def, _), _)| *def == r.elem_def)
        .filter(|((_, c), id)| match &t.sel {
            GrantSel::Whole => true,
            GrantSel::Ctors(ids) => ids.contains(id),
            GrantSel::Set(s) => r.sets.iter().any(|set| {
                schema.ids.sets.get(&(rel, set.name.clone())) == Some(s) && set.ctors.contains(c)
            }),
        })
        .map(|(_, &id)| Target {
            relation: RelationId(t.relation),
            constructor: ConstructorId(id),
        })
        .collect()
}

fn relation(schema: &Schema, id: u64) -> Option<RelId> {
    schema
        .ids
        .relations
        .iter()
        .find(|(_, x)| **x == id)
        .map(|(&r, _)| r)
}

impl Requirement {
    /// Returns what the requirement does, as `reads identities.EmailPass (hash)`, and the
    /// privilege it needs, as `read`.
    pub fn text(&self, schema: &Schema) -> (String, &'static str) {
        let at = |t: &Target| {
            let rel = relation(schema, t.relation.0);
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
