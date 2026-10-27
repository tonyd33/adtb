use super::*;

/// A constraint, or a check when `check`, that a migration keeps although its operators drop it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    pub module: String,
    pub name: String,
    pub check: bool,
}

impl Kept {
    fn kind(&self) -> &'static str {
        if self.check { "check" } else { "constraint" }
    }
}

impl Source {
    /// Put back each declaration of `kept` that `prior` holds and this source lacks, with the
    /// catalog id `prior` gives it.
    pub(super) fn keep(&mut self, prior: &Source, kept: &[Kept]) {
        let mut restored = false;
        for k in kept {
            let Some(i) = prior.find(k.kind(), &k.module, &k.name) else {
                continue;
            };
            if self.find(k.kind(), &k.module, &k.name).is_none() {
                self.push(&k.module, prior.decls[i].text.clone());
                restored = true;
            }
        }
        if restored {
            for (path, &id) in &prior.ids {
                self.ids.entry(path.clone()).or_insert(id);
            }
            self.reconcile_ids();
        }
    }
}
