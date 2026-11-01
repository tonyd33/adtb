use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use adtb_testkit::fixture::Fixture;

mod driver;
#[path = "../runtime/mod.rs"]
mod shared;

use shared::{Fields, fields, finish, message, take};

fn directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/query-fixtures")
}

fn read(name: &str) -> String {
    shared::read(&directory(), name)
}

fn fixture() -> Fixture {
    shared::fixture(&directory())
}

#[test]
fn runtime_query_cases() {
    let fixture = fixture();
    let manifest = read("original-cases.txt");
    let hash = manifest.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    assert_eq!(
        hash, ORIGINAL_HASH,
        "the original case manifest is unchanged"
    );
    let required: BTreeSet<_> = manifest.lines().collect();
    assert_eq!(
        required.len(),
        manifest.lines().count(),
        "original IDs are unique"
    );
    let mut paths: Vec<_> = std::fs::read_dir(directory())
        .expect("fixture directory is readable")
        .map(|e| e.expect("fixture entry is readable").path())
        .collect();
    paths.sort();
    let mut executed = BTreeSet::new();
    let mut failures = Vec::new();
    for path in paths {
        let file = path
            .file_name()
            .expect("fixture has a name")
            .to_str()
            .expect("UTF-8 fixture name");
        if matches!(file, "schema.adtb" | "data.txt" | "original-cases.txt") {
            continue;
        }
        if path.extension().is_none_or(|e| e != "case") {
            failures.push(format!("unknown fixture file {file}"));
            continue;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                failures.push(format!("{file}: {error}"));
                continue;
            }
        };
        if !text.starts_with("=== ") {
            failures.push(format!("{file}: missing case header"));
            continue;
        }
        for block in text[4..].split("\n=== ") {
            let label = block.lines().next().unwrap_or("empty case");
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let (name, mode, fields) = fields(block);
                assert!(executed.insert(name.to_string()), "duplicate case {name}");
                driver::run(&fixture, mode, fields);
            }));
            if let Err(error) = outcome {
                failures.push(format!("{file}/{label}: {}", message(error.as_ref())));
            }
        }
    }
    for name in required {
        if !executed.contains(name) {
            failures.push(format!("missing original case {name}"));
        }
    }
    eprintln!("runtime queries: {} cases executed", executed.len());
    assert!(
        failures.is_empty(),
        "runtime query failures:\n{}",
        failures.join("\n")
    );
}

const ORIGINAL_HASH: u64 = 0xce0b2c3608118e71;
