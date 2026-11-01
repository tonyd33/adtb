use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

mod driver;
mod outcome;
mod probe;
mod refusal;
#[path = "../runtime/mod.rs"]
mod shared;
mod stage;

fn directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/write-fixtures")
}

pub(super) fn run() -> (usize, usize) {
    let directory = directory();
    let manifest = shared::read(&directory, "original-cases.txt");
    let hash = manifest.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    assert_eq!(
        hash, ORIGINAL_HASH,
        "the original write manifest is unchanged"
    );
    let required: BTreeSet<_> = manifest.lines().collect();
    assert_eq!(
        required.len(),
        manifest.lines().count(),
        "unique original IDs"
    );
    let mut paths: Vec<_> = std::fs::read_dir(&directory)
        .expect("write fixture directory is readable")
        .map(|entry| entry.expect("write fixture entry is readable").path())
        .collect();
    paths.sort();
    let mut executed = BTreeSet::new();
    let mut families = BTreeSet::new();
    let mut failures = Vec::new();
    for path in paths {
        let file = path
            .file_name()
            .expect("write fixture has a name")
            .to_str()
            .expect("UTF-8 write fixture name");
        if matches!(file, "schema.adtb" | "data.txt" | "original-cases.txt") {
            continue;
        }
        if path.extension().is_none_or(|extension| extension != "case") {
            failures.push(format!("unknown write fixture file {file}"));
            continue;
        }
        let family = path
            .file_stem()
            .expect("write family has a name")
            .to_str()
            .expect("UTF-8 write family name");
        let mut current = format!("{family}/setup(schema.adtb,data.txt)");
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert!(
                families.insert(family.to_owned()),
                "duplicate family {family}"
            );
            let text = shared::read(&directory, file);
            assert!(text.starts_with("=== "), "a write family declares steps");
            let mut fixture = shared::fixture(&directory);
            for block in text[4..].split("\n=== ") {
                current = format!("{family}/{}", block.lines().next().unwrap_or("empty step"));
                let (name, mode, fields) = shared::fields(block);
                let id = format!("{family}/{name}");
                assert!(executed.insert(id.clone()), "duplicate step {id}");
                driver::Step::read(&fixture, mode, fields).run(&mut fixture);
            }
        }));
        if let Err(error) = outcome {
            failures.push(format!(
                "{file}/{current}: {}",
                shared::message(error.as_ref())
            ));
        }
    }
    for id in required {
        if !executed.contains(id) {
            failures.push(format!("missing original write step {id}"));
        }
    }
    assert!(
        failures.is_empty(),
        "runtime write failures:\n{}",
        failures.join("\n")
    );
    (families.len(), executed.len())
}

const ORIGINAL_HASH: u64 = 0x1e0d0cd71585a7ba;
