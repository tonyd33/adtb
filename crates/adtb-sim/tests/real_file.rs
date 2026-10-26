//! A database on a real file under the host's runtime, with no simulator.

use std::path::{Path, PathBuf};

use adtb::catalog::Catalog;
use adtb::storage::{OpenError, Options, Pager};
use adtb::verify;
use adtb_lang::schema::ValueDef;
use adtb_sys::os::{Backend, OsRuntime};

const SCHEMA: &str = r"module shop
add module shop
add type Customer = { id: Key customers, name: Text }
add relation customers : Customer key id generate sequence
add type Payment { id: Key payments, customer: Key customers }
  = Card { last4: Text }
  | Invoice { due: Text }
add relation payments : Payment key id generate uuidv7
";

fn options() -> Options {
    Options {
        page_size: 4096,
        log_block: 4096,
        segment_size: 256 * 1024,
        pool_frames: 64,
        atomic_page_writes: false,
        ..Options::new("shop.adt")
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("real_file")
        .join(name);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("clear {dir:?}: {e}"),
        _ => {}
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn runtime(dir: &Path) -> OsRuntime {
    OsRuntime::new(dir, Backend::Pool { threads: 4 })
}

fn has_relation(catalog: &Catalog, module: &str, name: &str) -> bool {
    let schema = catalog.schema();
    let m = schema.module_named(module).expect("the module");
    matches!(
        schema.module(m).values.get(name),
        Some(ValueDef::Relation(_))
    )
}

#[test]
fn a_migrated_database_reopens_from_its_files() {
    let dir = scratch("reopen");
    {
        let rt = runtime(&dir);
        let pager = Pager::create(rt.clone(), &options()).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        catalog
            .migrate(&pager, &txns, SCHEMA)
            .expect("the migration applies");
    }
    assert!(dir.join("shop.adt").is_file());
    let segments = std::fs::read_dir(dir.join("shop.adt-log"))
        .expect("the log directory")
        .count();
    assert!(segments > 0, "the log has a segment");

    let rt = runtime(&dir);
    let (pager, _) = Pager::open(rt.clone(), &options()).expect("reopen");
    let (catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
    assert_eq!(1, catalog.source().version);
    assert!(has_relation(&catalog, "shop", "customers"));
    assert!(has_relation(&catalog, "shop", "payments"));
    let report = verify::verify(&pager, &catalog.inventory(&txns.commit_log()));
    assert!(report.is_clean() && report.complete, "{report:#?}");
}

#[test]
fn a_database_is_created_once() {
    let dir = scratch("create-once");
    let rt = runtime(&dir);
    drop(Pager::create(rt.clone(), &options()).expect("create"));
    assert!(matches!(
        Pager::create(rt, &options()),
        Err(OpenError::Exists)
    ));
}

#[test]
fn a_missing_database_is_not_found() {
    let dir = scratch("missing");
    assert!(matches!(
        Pager::open(runtime(&dir), &options()),
        Err(OpenError::NotFound)
    ));
}
