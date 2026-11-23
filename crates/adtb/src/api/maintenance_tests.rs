use adtb_sys::os::{Backend, OsRuntime};
use adtb_testkit::scratch::Scratch;

use crate::catalog::Catalog;
use crate::key::Value;
use crate::storage::{self, PageKind, Pager};
use crate::test_world::grants::deploy_fields;

use super::super::{CreateOptions, Database, Log, OpenError, OpenOptions};

#[test]
fn a_migration_row_with_a_short_hash_refuses_the_open_of_a_file_with_no_scrub_cursor() {
    let scratch = Scratch::new("maintenance", "migrations");
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(1),
        pool_bytes: Some(4 * 1024 * 1024),
        ..OpenOptions::default()
    };
    let pager = Pager::create(rt.clone(), &options.storage("m.adt")).unwrap();
    let (catalog, txns) = Catalog::open(rt, &pager).unwrap();
    deploy_fields(
        &catalog,
        &pager,
        &txns,
        ("migrations", "Migration"),
        vec![
            ("version", Value::I64(1)),
            ("hash", Value::Bytes(vec![7; 31])),
            ("text", Value::Text("create relation t".into())),
            ("applied", Value::Instant(41)),
        ],
    );
    drop(catalog);
    drop(txns);
    pager.close();
    drop(pager);
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let detail = match Database::open_in(rt, "m.adt", &Log::default(), &options) {
        Err(OpenError::Corrupt { detail }) => detail,
        Err(other) => panic!("the open refuses as damage, not {other:?}"),
        Ok(_) => panic!("the open refuses"),
    };
    assert_eq!(
        "a row of catalog::migrations.Migration holds a hash of 31 bytes, not 32",
        detail
    );
}

#[test]
fn a_file_of_an_older_catalog_format_is_refused_at_open_naming_its_format() {
    let scratch = Scratch::new("maintenance", "format");
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(1),
        pool_bytes: Some(4 * 1024 * 1024),
        ..OpenOptions::default()
    };
    let pager = Pager::create(rt.clone(), &options.storage("f.adt")).unwrap();
    let (catalog, txns) = Catalog::open(rt, &pager).unwrap();
    let root = pager.catalog_root();
    let edit = storage::header_edit(1, PageKind::Catalog, 0, 0);
    pager
        .update(&[root], |p| {
            p[0][storage::FIRST_EDITABLE..storage::HEADER_LEN].copy_from_slice(&edit);
        })
        .unwrap();
    drop(catalog);
    drop(txns);
    pager.close();
    drop(pager);
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let detail = match Database::open_in(rt, "f.adt", &Log::default(), &options) {
        Err(OpenError::Corrupt { detail }) => detail,
        Err(other) => panic!("the open refuses the format, not {other:?}"),
        Ok(_) => panic!("the open refuses"),
    };
    assert_eq!(
        "the file is of catalog format 1; this engine opens only format 2",
        detail
    );
}
