use super::*;

/// The declarations a role may name: every declaration of a module the scope does not restrict,
/// and of a restricted module the types, relation variables and functions it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    restricted: BTreeSet<ModuleId>,
    types: BTreeSet<TypeId>,
    relations: BTreeSet<RelId>,
    functions: BTreeSet<FnId>,
}

impl Scope {
    /// Returns the scope restricting every user module of `schema` to `types`, `relations` and
    /// `functions`.
    pub(crate) fn of_user_modules(
        schema: &Schema,
        types: BTreeSet<TypeId>,
        relations: BTreeSet<RelId>,
        functions: BTreeSet<FnId>,
    ) -> Scope {
        Scope {
            restricted: (0..schema.modules.len() as u32)
                .map(ModuleId)
                .filter(|&m| !schema.module(m).engine)
                .collect(),
            types,
            relations,
            functions,
        }
    }

    fn type_open(&self, m: ModuleId, t: TypeId) -> bool {
        !self.restricted.contains(&m) || self.types.contains(&t)
    }

    fn value_open(&self, m: ModuleId, v: &ValueDef) -> bool {
        !self.restricted.contains(&m)
            || match v {
                ValueDef::Relation(r) => self.relations.contains(r),
                ValueDef::Function(f) => self.functions.contains(f),
                ValueDef::Constraint(_) | ValueDef::Check(_) | ValueDef::Aggregate(_) => false,
            }
    }
}

impl Schema {
    /// Returns the module named `name` that `scope` lets a name find.
    pub(super) fn module_named_in(&self, name: &str, scope: Option<&Scope>) -> Option<ModuleId> {
        (0..self.modules.len() as u32)
            .map(ModuleId)
            .find(|&m| self.module(m).name == name && self.module_open(m, scope))
    }

    fn module_open(&self, m: ModuleId, scope: Option<&Scope>) -> bool {
        let Some(s) = scope else {
            return true;
        };
        let def = self.module(m);
        !s.restricted.contains(&m)
            || def.types.values().any(|&t| s.type_open(m, t))
            || def.values.values().any(|v| s.value_open(m, v))
    }

    /// Returns the type `name` of module `m` that `scope` lets a name find.
    pub(crate) fn type_in(&self, m: ModuleId, name: &str, scope: Option<&Scope>) -> Option<TypeId> {
        let t = *self.module(m).types.get(name)?;
        scope.is_none_or(|s| s.type_open(m, t)).then_some(t)
    }

    /// Returns the value `name` of module `m` that `scope` lets a name find.
    pub(crate) fn value_in(
        &self,
        m: ModuleId,
        name: &str,
        scope: Option<&Scope>,
    ) -> Option<ValueDef> {
        let v = *self.module(m).values.get(name)?;
        scope.is_none_or(|s| s.value_open(m, &v)).then_some(v)
    }
}
