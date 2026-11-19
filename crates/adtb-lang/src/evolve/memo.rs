use std::collections::{HashMap, VecDeque};
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
            b.decls == src.decls && b.modules == src.modules && b.ahead == src.ahead_inputs()
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
            ahead: src.ahead_inputs(),
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

/// A memo of the module files a source and its clones parsed, by their text, emptied when it is
/// full.
#[derive(Clone, Default)]
pub(super) struct ParsedMemo(Arc<Mutex<HashMap<String, ast::SchemaFile>>>);

/// The most files one parse memo holds.
const PARSED_BOUND: usize = 256;

impl fmt::Debug for ParsedMemo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ParsedMemo")
    }
}

impl ParsedMemo {
    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, ast::SchemaFile>> {
        self.0
            .lock()
            .expect("no panic while the memo's lock is held: it only looks up and moves entries")
    }

    /// Returns the parse of module file `text`, as `parse_schema` with elisions gives it.
    pub(super) fn parsed(&self, text: &str) -> Result<ast::SchemaFile, Vec<Diag>> {
        if let Some(f) = self.entries().get(text) {
            return Ok(f.clone());
        }
        let f = parse_schema(text, Options { elisions: true }).ok()?;
        let mut entries = self.entries();
        if entries.len() >= PARSED_BOUND {
            entries.clear();
        }
        entries.insert(text.to_owned(), f.clone());
        Ok(f)
    }
}
