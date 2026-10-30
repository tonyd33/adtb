use std::collections::VecDeque;
use std::fmt;
use std::sync::Mutex;

use super::*;

/// The most schemas one memo holds.
pub(super) const BOUND: usize = 32;

struct Built {
    decls: Vec<SourceDecl>,
    modules: BTreeSet<String>,
    ahead: Vec<schema::Ahead>,
    schema: Arc<Schema>,
}

/// A bounded memo of the schemas a source and its clones built, most recently used first.
///
/// An entry is keyed on every input of the build except the built-in declarations, which are
/// fixed for the process.
#[derive(Clone, Default)]
pub(super) struct BuiltMemo(Arc<Mutex<VecDeque<Built>>>);

impl fmt::Debug for BuiltMemo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BuiltMemo")
    }
}

impl BuiltMemo {
    fn entries(&self) -> std::sync::MutexGuard<'_, VecDeque<Built>> {
        self.0
            .lock()
            .expect("no panic while the memo's lock is held: it only compares and moves entries")
    }

    pub(super) fn get(&self, src: &Source) -> Option<Arc<Schema>> {
        let mut entries = self.entries();
        let at = entries.iter().position(|b| {
            b.decls == src.decls && b.modules == src.modules && b.ahead == src.ahead
        })?;
        let hit = entries.remove(at)?;
        let schema = hit.schema.clone();
        entries.push_front(hit);
        Some(schema)
    }

    pub(super) fn put(&self, src: &Source, schema: Arc<Schema>) {
        let built = Built {
            decls: src.decls.clone(),
            modules: src.modules.clone(),
            ahead: src.ahead.clone(),
            schema,
        };
        let mut entries = self.entries();
        entries.push_front(built);
        entries.truncate(BOUND);
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries().len()
    }
}
