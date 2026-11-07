mod scripts;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Returns the files the `mod NAME;` lines of `file` load, honouring `#[path]`.
fn declared_modules(file: &Path, source: &str) -> Vec<PathBuf> {
    let dir = file.parent().expect("a source lies in a directory");
    let owner = file.file_stem().and_then(|stem| stem.to_str());
    let children = match owner {
        Some("main" | "mod") => dir.to_path_buf(),
        Some(stem) => dir.join(stem),
        None => panic!("{} has no file name", file.display()),
    };
    let mut path = None;
    let mut found = Vec::new();
    for line in source.lines().map(str::trim) {
        if let Some(attribute) = line.strip_prefix("#[path = \"") {
            path = attribute.strip_suffix("\"]");
            continue;
        }
        let declared = line
            .strip_prefix("pub ")
            .unwrap_or(line)
            .strip_prefix("mod ")
            .and_then(|rest| rest.strip_suffix(';'));
        if let Some(name) = declared {
            found.push(match path {
                Some(path) => dir.join(path),
                None if children.join(format!("{name}.rs")).exists() => {
                    children.join(format!("{name}.rs"))
                }
                None => children.join(name).join("mod.rs"),
            });
        }
        path = None;
    }
    found
}

fn rust_sources(dir: &Path, found: &mut BTreeSet<PathBuf>) {
    for entry in fs::read_dir(dir).expect("a source directory reads") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_sources(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.insert(path);
        }
    }
}

const BINARIES: [&str; 6] = [
    "execution",
    "export",
    "maintenance",
    "scripted",
    "server",
    "storage",
];

#[test]
fn every_source_of_the_binary_is_a_module() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut with_main = BTreeSet::new();
    for entry in fs::read_dir(&tests).expect("the tests directory reads") {
        let path = entry.expect("a directory entry").path();
        if path.join("main.rs").exists() {
            with_main.insert(path.file_name().expect("a directory name").to_owned());
        }
    }
    let listed: BTreeSet<_> = BINARIES.iter().map(|binary| (*binary).into()).collect();
    assert_eq!(with_main, listed);
    let mut reached_all = BTreeSet::new();
    for binary in BINARIES {
        let root = tests.join(binary);
        let mut reached = BTreeSet::new();
        let mut pending = vec![root.join("main.rs")];
        while let Some(file) = pending.pop() {
            let source = fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("{} does not read: {e}", file.display()));
            pending.extend(declared_modules(&file, &source));
            reached.insert(file);
        }
        let mut sources = BTreeSet::new();
        rust_sources(&root, &mut sources);
        let unreached: Vec<_> = sources.difference(&reached).collect();
        assert_eq!(unreached, Vec::<&PathBuf>::new(), "{binary}");
        reached_all.extend(reached);
    }
    for (binary, through_path) in [
        ("execution", "constraints/names.rs"),
        ("storage", "io_contract/faults.rs"),
    ] {
        let file = tests.join(binary).join(through_path);
        assert!(reached_all.contains(&file), "{through_path} is not reached");
    }
}
