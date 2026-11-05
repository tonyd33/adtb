mod api;
mod api_real_file;
mod arrow_ipc;
mod background_vacuum;
mod backup;
mod btree;
mod btree_threads;
mod bundles;
mod calls;
mod catalog;
mod checked_registration;
mod checks;
mod clog;
mod commands;
mod commit;
mod constraints;
mod crash_images;
mod create;
mod derived_index;
mod directory_damage;
mod dump;
mod dump_server;
mod dump_validate;
mod embedding;
mod exec;
mod faults;
mod files;
mod grant_rows;
mod heap;
mod heap_claims;
mod heap_kind;
mod heap_threads;
mod history;
mod host_fns;
mod io_contract;
mod isolation;
mod key_renames;
mod latches;
mod loads;
mod longkey;
mod lost_bitmap;
mod maintenance_faults;
mod open_damage;
mod oplog;
mod page_past_log_end;
mod pager;
mod past;
mod principal;
mod quarantine;
mod real_file;
mod registered;
mod reindex;
mod reindex_review;
mod replay;
mod scheduler;
mod scripts;
mod scrub;
mod server_adhoc;
mod server_authority;
mod server_binding;
mod server_commands;
mod server_files;
mod ssi;
mod system_writes;
mod tasks;
mod textual;
mod truncated_open;
mod update_concurrency;
mod vacuum;
mod vacuum_budget;
mod vacuum_progress;
mod vacuum_threads;
mod verify;
mod write;
mod write_states;

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

#[test]
fn every_source_of_the_binary_is_a_module() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sim");
    let mut reached = BTreeSet::new();
    let mut pending = vec![root.join("main.rs")];
    while let Some(file) = pending.pop() {
        let source = fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("{} does not read: {e}", file.display()));
        pending.extend(declared_modules(&file, &source));
        reached.insert(file);
    }
    for through_path in ["constraints/names.rs", "io_contract/faults.rs"] {
        assert!(
            reached.contains(&root.join(through_path)),
            "{through_path} is not reached"
        );
    }
    let mut sources = BTreeSet::new();
    rust_sources(&root, &mut sources);
    let unreached: Vec<_> = sources.difference(&reached).collect();
    assert_eq!(unreached, Vec::<&PathBuf>::new());
}
