use std::sync::LazyLock;

use super::{ENGINE_SRC, PRELUDE_SRC};
use crate::ast;
use crate::parse::{self, Options};
use crate::span::FileId;

/// The prelude and the engine's own module files, parsed once.
pub(super) static BUILT_IN: LazyLock<Vec<(FileId, ast::SchemaFile)>> =
    LazyLock::new(|| built_in(PRELUDE_SRC, ENGINE_SRC));

/// Returns the prelude and the module files of the engine's declarations `engine`, parsed.
pub(super) fn built_in(prelude: &str, engine: &str) -> Vec<(FileId, ast::SchemaFile)> {
    let prelude = parse::parse_schema(prelude, Options::default())
        .ok()
        .expect("the prelude parses");
    let mut all = vec![(FileId::PRELUDE, prelude)];
    all.extend(
        engine_files(engine)
            .into_iter()
            .map(|f| (FileId::ENGINE, f)),
    );
    all
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
