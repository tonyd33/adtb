use super::*;

/// The write state of each constructor that is not enabled, by catalog id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct States {
    relations: BTreeMap<(CatRelId, CatCtorId), Stage>,
    values: BTreeMap<CatCtorId, Stage>,
}

impl States {
    /// Set the write state of constructor `ctor` in relation variable `rel`.
    pub fn set_relation(&mut self, rel: CatRelId, ctor: CatCtorId, stage: Stage) {
        self.relations.insert((rel, ctor), stage);
    }

    /// Set the write state of value-level constructor `ctor`.
    pub fn set_value(&mut self, ctor: CatCtorId, stage: Stage) {
        self.values.insert(ctor, stage);
    }
}

/// The catalog ids of a built schema, given the schema with its ids in declaration order.
pub(crate) type AssignIds<'a> = &'a dyn Fn(&Schema) -> CatalogIds;

/// Returns the schema [`build_ahead`] builds, its catalog ids those `ids` returns and its write
/// states `states`.
pub fn build_with(
    files: &[(FileId, ast::SchemaFile)],
    ahead: &[Ahead],
    ids: &dyn Fn(&Schema) -> CatalogIds,
    states: &States,
) -> Result<Schema, Vec<FileDiag>> {
    Ok(build_ahead(files, ahead)?.relabelled(ids, states))
}

impl Schema {
    /// Returns the write state of each constructor that is not enabled, by catalog id; one
    /// whose ids the schema lacks is left out.
    pub fn states(&self) -> States {
        let ctor = |t: TypeId, c: u32| self.ty(t).ctor_at(c).and_then(|c| self.ids.ctor_id(c));
        let relations = self
            .stages
            .iter()
            .filter_map(|(&(r, c), &s)| {
                let rel = self.ids.relation_id(r)?;
                Some(((rel, ctor(self.relation(r).elem_def, c)?), s))
            })
            .collect();
        let values = self
            .value_stages
            .iter()
            .filter_map(|(&(t, c), &s)| Some((ctor(t, c)?, s)))
            .collect();
        States { relations, values }
    }

    /// Returns the schema [`build_with`] builds from the files and ahead relations this schema was
    /// built from.
    ///
    /// Preconditions: the schema is one [`build_ahead`] returned.
    pub(crate) fn relabelled(&self, ids: AssignIds, states: &States) -> Schema {
        let ids = ids(self);
        let mut stages = BTreeMap::new();
        for (r, rel) in ids.relation_ids() {
            for c in self.ty(self.relation(r).elem_def).ctor_refs() {
                let stage = ids
                    .ctor_id(c)
                    .and_then(|ctor| states.relations.get(&(rel, ctor)));
                if let Some(&s) = stage
                    && s != Stage::Enabled
                {
                    stages.insert((r, c.index()), s);
                }
            }
        }
        let mut value_stages = BTreeMap::new();
        for (c, ctor) in ids.ctor_ids() {
            if let Some(&s) = states.values.get(&ctor)
                && s != Stage::Enabled
            {
                value_stages.insert((c.def(), c.index()), s);
            }
        }
        Schema {
            defs: self.defs.clone(),
            ids,
            stages,
            value_stages,
            temps: BTreeMap::new(),
        }
    }
}
