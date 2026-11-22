use super::*;
use crate::ids::{CatCtorId, CatRelId, keyed};

impl Model {
    /// Returns the source the model holds: its declarations printed, a name of dropped objects as
    /// written, and each object of them by its key with the id the model holds, of each named
    /// one alone while they do not build.
    pub(super) fn source(&self) -> Source {
        Source {
            decls: self.printed().expect("the declarations print"),
            ids: self.ids().unwrap_or_else(|_| self.named_keys()),
            modules: self.modules.clone(),
            installed: self.installed.clone(),
            packages: self.packages.clone(),
            grants: self.grants.clone(),
            states: self
                .states
                .iter()
                .map(|(&(r, c), &s)| ((CatRelId::from_raw(r), CatCtorId::from_raw(c)), s))
                .collect(),
            value_states: self
                .value_states
                .iter()
                .map(|(&c, &s)| (CatCtorId::from_raw(c), s))
                .collect(),
            version: self.version,
            log: self.logged.clone(),
            next: self.next.clone(),
            ..Source::default()
        }
    }

    /// Returns the catalog id of each object of the declarations, by its key.
    ///
    /// Fail when the declarations do not build.
    pub(super) fn ids(&self) -> Result<BTreeMap<ObjectKey, u64>, String> {
        let (built, _) = self
            .built(&[], &self.ahead_inputs())
            .map_err(|ds| ds[0].diag.message.clone())?;
        let schema = built.schema();
        let objs = built.objects();
        let ids = schema.ids();
        let keys = keyed(&objs, &|s| ids.slot_id(s))
            .ok_or("an object of the built declarations has no id")?;
        Ok(keys.into_iter().collect())
    }
}
