//! The in-process API over a real file under the host's runtime, with no simulator.

use std::path::PathBuf;

use adtb::{CallOptions, CreateOptions, Log, OpenError, OpenOptions, TxnCallOptions, TxnOptions};
use adtb_lang::textual::print;

const SHOP: &str = r"module shop
add module shop
add type Customer = { id: Key customers, name: Text }
add relation customers : Customer key id generate sequence
";

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("api_real_file")
        .join(name);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("clear {dir:?}: {e}"),
        _ => {}
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn options() -> OpenOptions {
    OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(2),
        ..OpenOptions::default()
    }
}

fn names(db: &adtb::Database) -> Vec<String> {
    let mut out = db
        .query_text(
            "from c in customers select c.name",
            &[],
            &CallOptions::default(),
        )
        .expect("the names");
    let rows = out.remove(0);
    let form = rows.form().clone();
    rows.map(|r| print(&r.expect("a row"), &form).expect("a row prints"))
        .collect()
}

#[test]
fn a_database_written_through_the_api_reopens_with_its_rows() {
    let path = scratch("reopen").join("shop.adt");
    let log = Log::new([SHOP]);
    {
        let db = adtb::open(&path, &log, &options()).expect("create");
        db.query_text(
            r#"insert customers { name: "ann" }"#,
            &[],
            &CallOptions::default(),
        )
        .expect("an insert");
        let mut t = db.begin(&TxnOptions::default()).expect("begin");
        t.query_text(
            r#"insert customers { name: "bob" }"#,
            &[],
            &TxnCallOptions::default(),
        )
        .expect("an insert in a transaction");
        t.commit().expect("commit");
        db.close().expect("close");
    }
    assert!(path.is_file());
    assert!(path.with_file_name("shop.adt-log").is_dir());
    let db = adtb::open(&path, &log, &OpenOptions::default()).expect("reopen");
    assert_eq!(vec![r#""ann""#, r#""bob""#], names(&db));
}

#[test]
fn a_file_open_in_this_process_is_not_opened_again() {
    let path = scratch("twice").join("shop.adt");
    let log = Log::new([SHOP]);
    let db = adtb::open(&path, &log, &options()).expect("create");
    let again = adtb::open(&path, &log, &options());
    assert!(matches!(again, Err(OpenError::AlreadyOpen)), "{again:?}");
    let clone = db.clone();
    drop(db);
    assert!(
        matches!(
            adtb::open(&path, &log, &options()),
            Err(OpenError::AlreadyOpen)
        ),
        "a clone keeps the file open"
    );
    drop(clone);
    adtb::open(&path, &log, &options()).expect("open after the last handle closes");
}

#[test]
fn a_missing_file_is_not_found_without_create() {
    let path = scratch("missing").join("shop.adt");
    let got = adtb::open(&path, &Log::new([SHOP]), &OpenOptions::default());
    assert!(matches!(got, Err(OpenError::NotFound)), "{got:?}");
    assert!(!path.exists(), "open without create made the file");
}
