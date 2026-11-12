use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ordinary,
    Registration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Right {
    Read {
        target: Target,
        fields: BTreeSet<CatFieldId>,
    },
    Insert(Target),
    Update {
        target: Target,
        fields: BTreeSet<CatFieldId>,
    },
    Delete(Target),
    Retag {
        relation: CatRelId,
        from: CatCtorId,
        to: CatCtorId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveGrant {
    pub right: Right,
    pub grant_option: bool,
}

/// An applied authority snapshot over the identities of the checked schema.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectiveGrants {
    pub relations: Vec<EffectiveGrant>,
    pub read_files: bool,
}

impl Requirement {
    fn on_temp(&self) -> bool {
        match self {
            Requirement::ReadConstructor(t)
            | Requirement::ReadField(t, _)
            | Requirement::Insert(t)
            | Requirement::Delete(t)
            | Requirement::UpdateField(t, _) => *t == TEMP_TARGET,
            Requirement::Retag { relation, .. } => *relation == TEMP_TARGET.relation,
            Requirement::ReadFiles => false,
        }
    }

    /// Returns the relation the requirement names, by its catalog id.
    pub fn relation(&self) -> Option<CatRelId> {
        match self {
            Requirement::ReadConstructor(t)
            | Requirement::ReadField(t, _)
            | Requirement::Insert(t)
            | Requirement::Delete(t)
            | Requirement::UpdateField(t, _) => Some(t.relation),
            Requirement::Retag { relation, .. } => Some(*relation),
            Requirement::ReadFiles => None,
        }
    }
}

impl Requirements {
    /// Returns every requirement the explicit effective snapshot does not cover.
    pub fn missing(&self, grants: &EffectiveGrants, mode: Mode) -> Vec<Requirement> {
        self.required
            .iter()
            .filter(|r| {
                if matches!(r, Requirement::ReadFiles) {
                    return !grants.read_files;
                }
                if r.on_temp() {
                    return false;
                }
                !grants.relations.iter().any(|g| {
                    (mode == Mode::Ordinary || g.grant_option) && self.covered(r, &g.right)
                })
            })
            .cloned()
            .collect()
    }

    fn covered(&self, requirement: &Requirement, right: &Right) -> bool {
        match (requirement, right) {
            (Requirement::ReadField(a, f), Right::Read { target: b, fields }) => {
                a == b && (fields.contains(f) || self.keys.contains(&(*a, *f)))
            }
            (Requirement::ReadConstructor(a), Right::Read { target: b, .. })
            | (Requirement::Insert(a), Right::Insert(b))
            | (Requirement::Delete(a), Right::Delete(b)) => a == b,
            (Requirement::UpdateField(a, f), Right::Update { target: b, fields }) => {
                a == b && fields.contains(f)
            }
            (
                Requirement::Retag {
                    relation: a,
                    from: b,
                    to: c,
                },
                Right::Retag {
                    relation: x,
                    from: y,
                    to: z,
                },
            ) => (a, b, c) == (x, y, z),
            _ => false,
        }
    }
}
