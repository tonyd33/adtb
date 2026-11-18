use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::step::{self, say};
use crate::workspace::Workspace;

pub const STEP: &str = "tree-sitter";

const GRAMMAR: &str = "tools/tree-sitter-adtb";

/// The hand-written external scanner.
const SCANNER: &str = "scanner.c";

/// Fail when the editor grammar's `src/` differs from what its `grammar.js` generates, then run
/// its corpus and highlight tests.
pub fn run(workspace: &Workspace) -> Result<(), String> {
    let grammar = workspace.root.join(GRAMMAR);
    let scratch = workspace.target.join("tree-sitter");
    let generated = scratch.join("generated");
    if generated.exists() {
        std::fs::remove_dir_all(&generated)
            .map_err(|e| format!("{STEP}: removing {}: {e}", generated.display()))?;
    }
    let tree_sitter = || {
        let mut command = workspace.command("tree-sitter");
        command.env("TREE_SITTER_LIBDIR", scratch.join("lib"));
        command
    };
    step::run(
        STEP,
        tree_sitter()
            .current_dir(&grammar)
            .args(["generate", "--js-runtime", "native", "--output"])
            .arg(&generated),
    )?;
    let differences = differences(&generated, &grammar.join("src"))?;
    if !differences.is_empty() {
        for difference in &differences {
            say(format_args!("{GRAMMAR}/src/{difference}"))?;
        }
        let files = if differences.len() == 1 {
            "file"
        } else {
            "files"
        };
        return Err(format!(
            "{STEP} failed: {GRAMMAR}/src and what grammar.js generates into {} differ in {} {files}",
            generated.display(),
            differences.len()
        ));
    }
    step::run(STEP, tree_sitter().args(["test", "-p"]).arg(&grammar))
}

/// Returns a line for each file other than the scanner under `generated` or `checked_in` that
/// the other lacks or holds with other bytes, as its path relative to them.
fn differences(generated: &Path, checked_in: &Path) -> Result<Vec<String>, String> {
    let generated_files = files(generated)?;
    let checked_in_files = files(checked_in)?;
    let mut differences = Vec::new();
    for file in generated_files.union(&checked_in_files) {
        let shown = file.display();
        if !checked_in_files.contains(file) {
            differences.push(format!("{shown}: missing, generated and not checked in"));
        } else if !generated_files.contains(file) {
            differences.push(format!("{shown}: extra, checked in and not generated"));
        } else if read(&generated.join(file))? != read(&checked_in.join(file))? {
            differences.push(format!("{shown}: stale, differs from the generated file"));
        }
    }
    Ok(differences)
}

/// Returns the paths relative to `dir` of the files under it, the scanner aside.
fn files(dir: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let mut found = BTreeSet::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let entries = std::fs::read_dir(&next)
            .map_err(|e| format!("{STEP}: reading {}: {e}", next.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{STEP}: reading {}: {e}", next.display()))?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|e| format!("{STEP}: reading {}: {e}", path.display()))?;
            if kind.is_dir() {
                pending.push(path);
            } else if entry.file_name() != SCANNER {
                let relative = path.strip_prefix(dir).map_err(|e| {
                    format!(
                        "{STEP}: {} is outside {}: {e}",
                        path.display(),
                        dir.display()
                    )
                })?;
                found.insert(relative.to_path_buf());
            }
        }
    }
    Ok(found)
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{STEP}: reading {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_but_the_scanner_is_compared_from_both_sides() {
        let scratch = std::env::temp_dir().join(format!("xtask-grammar-{}", std::process::id()));
        let generated = scratch.join("generated");
        let checked_in = scratch.join("src");
        let files = [
            (&generated, "parser.c", "a"),
            (&checked_in, "parser.c", "a"),
            (&generated, "tree_sitter/parser.h", "a"),
            (&checked_in, "tree_sitter/parser.h", "b"),
            (&generated, "grammar.json", "a"),
            (&checked_in, "node-types.json", "a"),
            (&checked_in, "scanner.c", "a"),
            (&generated, "tree_sitter/scanner.c", "a"),
        ];
        for (dir, file, text) in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().expect("a file under a directory"))
                .expect("the scratch tree is written");
            std::fs::write(&path, text).expect("the scratch tree is written");
        }
        let found = differences(&generated, &checked_in);
        std::fs::remove_dir_all(&scratch).expect("the scratch tree is removed");
        assert_eq!(
            found.expect("the scratch tree is read"),
            [
                "grammar.json: missing, generated and not checked in",
                "node-types.json: extra, checked in and not generated",
                "tree_sitter/parser.h: stale, differs from the generated file",
            ]
        );
    }
}
