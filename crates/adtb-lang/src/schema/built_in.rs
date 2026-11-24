use std::sync::{Arc, LazyLock, OnceLock};

use super::{ENGINE_SRC, PRELUDE_SRC, Schema};
use crate::ast;
use crate::ids::CatalogIds;
use crate::parse::{self, Options};
use crate::span::FileId;

/// The prelude and the engine's own module files, parsed once.
pub(super) static BUILT_IN: LazyLock<BuiltIn> =
    LazyLock::new(|| BuiltIn::new(PRELUDE_SRC, ENGINE_SRC));

/// The prelude and the engine's module files, parsed, with the fixed ids of the engine's objects
/// once a build over them computed them.
pub(super) struct BuiltIn {
    files: Vec<(FileId, ast::SchemaFile)>,
    engine_ids: OnceLock<Arc<CatalogIds>>,
}

impl BuiltIn {
    /// Returns the prelude `prelude` and the module files of the engine's declarations `engine`,
    /// parsed.
    pub(super) fn new(prelude: &str, engine: &str) -> BuiltIn {
        let prelude = parse::parse_schema(prelude, Options::default())
            .ok()
            .expect("the prelude parses");
        let mut files = vec![(FileId::PRELUDE, prelude)];
        files.extend(
            engine_files(engine)
                .into_iter()
                .map(|f| (FileId::ENGINE, f)),
        );
        BuiltIn {
            files,
            engine_ids: OnceLock::new(),
        }
    }

    pub(super) fn files(&self) -> &[(FileId, ast::SchemaFile)] {
        &self.files
    }

    /// Returns the engine's fixed ids in `schema`, a build over these files.
    ///
    /// Preconditions: every build over these files gives each engine object the position it
    /// gives it in every other.
    pub(super) fn engine_ids(&self, schema: &Schema) -> Arc<CatalogIds> {
        let ids = self
            .engine_ids
            .get_or_init(|| Arc::new(crate::ids::engine(schema)))
            .clone();
        #[cfg(any(test, feature = "testing"))]
        if crate::sampled::ENGINE_IDS.due() {
            assert_eq!(
                *ids,
                crate::ids::engine(schema),
                "the engine's fixed ids are the same in every build over the built-in files"
            );
        }
        ids
    }
}

/// Returns the module files of the engine's declarations `engine`.
fn engine_files(engine: &str) -> Vec<ast::SchemaFile> {
    let items = parse::parse_example(engine, Options::default())
        .ok()
        .expect("the engine's declarations parse");
    let mut files: Vec<ast::SchemaFile> = Vec::new();
    for item in items {
        match item {
            ast::ExampleItem::Module(m) => files.push(ast::SchemaFile {
                module: m,
                decls: Vec::new(),
            }),
            ast::ExampleItem::Decl(d) => files
                .last_mut()
                .expect("the engine's declarations start with a module")
                .decls
                .push(d),
            _ => panic!("the engine's declarations hold only modules and declarations"),
        }
    }
    files
}
