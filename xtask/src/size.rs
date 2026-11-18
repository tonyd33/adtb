use std::path::{Path, PathBuf};

use crate::step::say;

pub const STEP: &str = "size";

/// The most lines a non-test Rust source under `crates/` may have.
const LINE_LIMIT: usize = 1500;

/// Fail when a Rust source under `crates/` passes the line limit, test sources aside: a file
/// under a `tests` directory, `tests.rs` and `*_tests.rs`.
pub fn run(root: &Path) -> Result<(), String> {
    let mut sources = Vec::new();
    collect(&root.join("crates"), &mut sources)?;
    let mut over = 0;
    for file in sources {
        let text =
            std::fs::read(&file).map_err(|e| format!("{STEP}: reading {}: {e}", file.display()))?;
        let lines = text.iter().filter(|&&byte| byte == b'\n').count();
        if lines > LINE_LIMIT {
            let shown = file.strip_prefix(root).unwrap_or(&file);
            say(format_args!(
                "{} has {lines} lines, over {LINE_LIMIT}",
                shown.display()
            ))?;
            over += 1;
        }
    }
    if over == 0 {
        Ok(())
    } else {
        Err(format!(
            "{STEP} failed: {over} non-test sources under crates/ pass {LINE_LIMIT} lines"
        ))
    }
}

fn collect(dir: &Path, sources: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("{STEP}: reading {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{STEP}: reading {}: {e}", dir.display()))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|e| format!("{STEP}: reading {}: {e}", path.display()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if kind.is_dir() {
            if name != "tests" && name != "target" {
                collect(&path, sources)?;
            }
        } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
            sources.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sources_and_build_output_are_not_counted() {
        let crates = std::env::temp_dir().join(format!("xtask-size-{}", std::process::id()));
        let files = [
            "a/src/lib.rs",
            "a/src/exec/mod.rs",
            "a/src/notes.md",
            "a/src/tests.rs",
            "a/src/exec/scan_tests.rs",
            "a/tests/area/main.rs",
            "a/src/tests/case.rs",
            "a/target/debug/build/out.rs",
        ];
        for file in files {
            let path = crates.join(file);
            std::fs::create_dir_all(path.parent().expect("a file under a directory"))
                .expect("the scratch tree is written");
            std::fs::write(&path, "").expect("the scratch tree is written");
        }
        let mut sources = Vec::new();
        let collected = collect(&crates, &mut sources);
        std::fs::remove_dir_all(&crates).expect("the scratch tree is removed");
        collected.expect("the scratch tree is read");
        sources.sort();
        assert_eq!(
            sources,
            [
                crates.join("a/src/exec/mod.rs"),
                crates.join("a/src/lib.rs")
            ]
        );
    }
}
