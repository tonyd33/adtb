use adtb_sys::os::{Backend, OsRuntime};
use adtb_testkit::scratch::Scratch;

use crate::catalog::Catalog;
use crate::key::Value;
use crate::storage::Pager;
use crate::test_world::grants::deploy_fields;

use super::super::{CreateOptions, Database, Log, OpenError, OpenOptions};

#[test]
fn a_negative_count_setting_stored_refuses_the_open_naming_the_setting_and_value() {
    let scratch = Scratch::new("settings", "negative");
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(1),
        pool_bytes: Some(4 * 1024 * 1024),
        ..OpenOptions::default()
    };
    let pager = Pager::create(rt.clone(), &options.storage("s.adt")).unwrap();
    let (catalog, txns) = Catalog::open(rt, &pager).unwrap();
    deploy_fields(
        &catalog,
        &pager,
        &txns,
        ("settings", "RetryLimit"),
        vec![("id", Value::I64(451)), ("retries", Value::I64(-1))],
    );
    drop(catalog);
    drop(txns);
    pager.close();
    drop(pager);
    let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
    let detail = match Database::open_in(rt, "s.adt", &Log::default(), &options) {
        Err(OpenError::Corrupt { detail }) => detail,
        Err(other) => panic!("the open refuses as damage, not {other:?}"),
        Ok(_) => panic!("the open refuses"),
    };
    assert_eq!(
        "a row of catalog::settings.RetryLimit holds `retries` outside its range",
        detail
    );
}
