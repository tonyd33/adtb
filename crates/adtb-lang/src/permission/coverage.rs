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
        fields: BTreeSet<FieldId>,
    },
    Insert(Target),
    Update {
        target: Target,
        fields: BTreeSet<FieldId>,
    },
    Delete(Target),
    Retag {
        relation: RelationId,
        from: ConstructorId,
        to: ConstructorId,
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

impl Requirements {
    /// Returns every requirement the explicit effective snapshot does not cover.
    pub fn missing(&self, grants: &EffectiveGrants, mode: Mode) -> Vec<Requirement> {
        self.required
            .iter()
            .filter(|r| {
                if matches!(r, Requirement::ReadFiles) {
                    return !grants.read_files;
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
