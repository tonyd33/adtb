//! The in-process API over a real file under the host's runtime, with no simulator.

use std::path::{Path, PathBuf};

use adtb::{
    CallOptions, CreateOptions, Database, Log, OpenError, OpenOptions, TxnCallOptions, TxnOptions,
};
use adtb_lang::textual::print;
use adtb_sys::os::{Backend, OsRuntime};

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

/// Returns the files under `dir` this process holds open.
fn open_under(dir: &Path) -> Vec<PathBuf> {
    let dir = dir.canonicalize().expect("the directory");
    std::fs::read_dir("/proc/self/fd")
        .expect("the process's descriptors")
        .filter_map(|e| std::fs::read_link(e.expect("a descriptor").path()).ok())
        .filter(|p| p.starts_with(&dir))
        .collect()
}

fn options() -> OpenOptions {
    OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(2),
        ..OpenOptions::default()
    }
}

fn names(db: &Database) -> Vec<String> {
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
        t.commit(None).expect("commit");
        db.close().expect("close");
    }
    assert!(path.is_file());
    assert!(path.with_file_name("shop.adt-log").is_dir());
    let db = adtb::open(&path, &log, &OpenOptions::default()).expect("reopen");
    assert_eq!(vec![r#""ann""#, r#""bob""#], names(&db));
}

fn answers(db: &Database, text: &str) -> Vec<String> {
    let rows = db
        .query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"))
        .remove(0);
    let form = rows.form().clone();
    rows.map(|r| print(&r.expect("a row"), &form).expect("a row prints"))
        .collect()
}

#[test]
fn a_stored_view_of_a_scalar_or_nested_element_type_reopens_on_a_real_file() {
    let path = scratch("scalar-view").join("shop.adt");
    const VIEWS: &str = r#"module shop
define fn names () -> Set Text = from c in customers select c.name
store names
define fn sizes () -> Set I64 = from c in customers select count(d for d in customers)
store sizes
define fn nested () -> Set (Set Text) =
  from c in customers where c.name == "ann" select (from d in customers select d.name)
store nested
"#;
    {
        let db = adtb::open(&path, &Log::new([SHOP]), &options()).expect("create");
        db.query_text(
            r#"insert customers { name: "ann" }"#,
            &[],
            &CallOptions::default(),
        )
        .expect("the insert");
    }
    let log = Log::new([SHOP, VIEWS]);
    for _ in 0..2 {
        let db = adtb::open(&path, &log, &options()).expect("the views apply and reopen");
        assert_eq!(vec![r#""ann""#], answers(&db, "names()"));
        assert_eq!(vec!["1"], answers(&db, "sizes()"));
        assert_eq!(vec![r#"["ann"]"#], answers(&db, "nested()"));
    }
}

#[test]
fn the_migration_storing_a_view_of_text_reopens_on_a_real_file() {
    const PETS: &str = "module pets
add module pets
add type Pet { id: Key pets } = Cat { name: Text } | Dog { age: I64 }
add relation pets : Pet key id generate sequence
";
    const CATS: &str = "module pets
define fn cats () -> Set Text = from c in pets.Cat select c.name
store cats
";
    let path = scratch("cats").join("pets.adt");
    {
        let db = adtb::open(&path, &Log::new([PETS]), &options()).expect("create");
        db.query_text(
            r#"insert pets.Cat { name: "tom" }"#,
            &[],
            &CallOptions::default(),
        )
        .expect("the insert");
    }
    let log = Log::new([PETS, CATS]);
    for _ in 0..2 {
        let db = adtb::open(&path, &log, &options()).expect("the view applies and reopens");
        assert_eq!(vec![r#""tom""#], answers(&db, "cats()"));
    }
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

#[test]
fn a_closed_database_holds_no_file_open() {
    let dir = scratch("close");
    let path = dir.join("shop.adt");
    let log = Log::new([SHOP]);
    let db = adtb::open(&path, &log, &options()).expect("create");
    db.query_text(
        r#"insert customers { name: "ann" }"#,
        &[],
        &CallOptions::default(),
    )
    .expect("an insert");
    let clone = db.clone();
    db.close().expect("close");
    assert_eq!(
        Vec::<PathBuf>::new(),
        open_under(&dir),
        "close closes the files whatever clones remain"
    );
    clone.close().expect("close");
    let db = adtb::open(&path, &log, &OpenOptions::default()).expect("reopen");
    assert_eq!(vec![r#""ann""#], names(&db));
}

#[test]
fn a_failed_open_holds_no_file_open() {
    let dir = scratch("refused");
    let rt = OsRuntime::new(&dir, Backend::Pool { threads: 2 });
    let refused = Log::new([SHOP, "module shop\nremove relation missing"]);
    let got = Database::open_in(rt.clone(), "shop.adt", &refused, &options());
    assert!(
        matches!(got, Err(OpenError::MigrationRefused { version: 2, .. })),
        "{got:?}"
    );
    assert_eq!(Vec::<PathBuf>::new(), open_under(&dir));
    drop(rt);
}

#[test]
fn a_refused_create_leaves_no_file_behind() {
    let dir = scratch("refused-create");
    let path = dir.join("shop.adt");
    let refused = Log::new([SHOP, "module shop\nremove relation missing"]);
    let got = adtb::open(&path, &refused, &options()).map(drop);
    assert!(
        matches!(got, Err(OpenError::MigrationRefused { version: 2, .. })),
        "{got:?}"
    );
    let left: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the directory")
        .map(|e| e.expect("an entry").path())
        .collect();
    assert_eq!(
        Vec::<PathBuf>::new(),
        left,
        "a refused create removes what it made"
    );
    adtb::open(&path, &Log::new([SHOP]), &options()).expect("a later create succeeds");
}

#[test]
fn a_refused_open_of_an_existing_file_keeps_it() {
    let dir = scratch("refused-existing");
    let path = dir.join("shop.adt");
    let log = Log::new([SHOP]);
    adtb::open(&path, &log, &options())
        .expect("create")
        .close()
        .expect("close");
    let refused = Log::new([SHOP, "module shop\nremove relation missing"]);
    let got = adtb::open(&path, &refused, &options()).map(drop);
    assert!(
        matches!(got, Err(OpenError::MigrationRefused { .. })),
        "{got:?}"
    );
    assert!(
        path.is_file(),
        "an open that created nothing removes nothing"
    );
    let db = adtb::open(&path, &log, &OpenOptions::default()).expect("reopen");
    assert!(names(&db).is_empty());
}

/// A query that runs one fixpoint round per number up to ten million, far longer than any test
/// waits for it.
const LONG: &str =
    "fix n = [0] union (from x in n where x < 10000000 select x + 1) in count(x for x in n)";

/// Returns what [`LONG`] returns in a new database in scratch directory `name`, under the
/// options `opts` makes, while `stop` runs on another thread.
fn stopped(
    name: &str,
    opts: impl FnOnce(&OsRuntime) -> CallOptions,
    stop: impl FnOnce() + Send,
) -> Result<Vec<adtb::DynRows>, adtb::Error> {
    let dir = scratch(name);
    let rt = OsRuntime::new(&dir, Backend::Pool { threads: 2 });
    let db =
        Database::open_in(rt.clone(), "shop.adt", &Log::new([SHOP]), &options()).expect("create");
    let opts = CallOptions {
        fix_rounds: Some(usize::MAX),
        ..opts(&rt)
    };
    std::thread::scope(|s| {
        s.spawn(stop);
        db.query_text(LONG, &[], &opts)
    })
}

fn rejected(got: &Result<Vec<adtb::DynRows>, adtb::Error>) -> Option<&adtb::RejectReason> {
    match got {
        Err(adtb::Error::Rejected { reason }) => Some(reason),
        _ => None,
    }
}

#[test]
fn cancelling_a_running_call_from_another_thread_stops_it_promptly() {
    let tok = adtb::CancelToken::new();
    let other = tok.clone();
    let got = stopped(
        "cancel",
        |_| CallOptions {
            cancel: Some(tok.clone()),
            ..CallOptions::default()
        },
        move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            other.cancel();
        },
    );
    assert_eq!(
        Some(&adtb::RejectReason::Cancelled),
        rejected(&got),
        "{got:?}"
    );
}

#[test]
fn a_running_call_past_its_deadline_stops_with_deadline_exceeded() {
    use adtb::runtime::Clock;
    let got = stopped(
        "deadline",
        |rt| CallOptions {
            deadline: Some(rt.wall() + std::time::Duration::from_millis(50)),
            ..CallOptions::default()
        },
        || {},
    );
    assert_eq!(
        Some(&adtb::RejectReason::DeadlineExceeded),
        rejected(&got),
        "{got:?}"
    );
}

#[test]
fn a_statement_past_its_limit_stops_with_statement_limit() {
    let after = std::time::Duration::from_millis(50);
    let got = stopped(
        "limit",
        |_| CallOptions {
            limit: Some(after),
            ..CallOptions::default()
        },
        || {},
    );
    assert_eq!(
        Some(&adtb::RejectReason::StatementLimit {
            limit: adtb::Limit::Time { after }
        }),
        rejected(&got),
        "{got:?}"
    );
}

const SUM: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text }
  | Google    { gid: Text }
  | Apple     { sub: Text }
add relation identities : Identity key id generate sequence
  index (user) in (Google | Apple)
";

const SIDES: &str = "from i in identities.(EmailPass | Google) select i.id";
const ALL: &str = "from i in identities select i.id";
const ROWS: &str = "from i in identities select i";
const BY_USER: &str = "from i in identities.Google where i.user == users.key(1) select i.gid";
const INSERT: &str =
    r#"insert identities.Google { id: identities.key(9), user: users.key(1), gid: "g-2" }"#;

fn quarantined(db: &Database, text: &str) -> String {
    match db.query_text(text, &[], &CallOptions::default()) {
        Err(adtb::Error::Fatal {
            reason: adtb::FatalReason::Quarantined { what },
        }) => what,
        other => panic!("{text}: {other:?}"),
    }
}

fn runs(db: &Database, text: &str) {
    db.query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("{text}: {e:?}"));
}

/// Returns a seeded file under a new directory with the page `pick` picks flipped.
fn damaged(name: &str, pick: impl FnOnce(&Database) -> adtb::storage::Pid) -> PathBuf {
    let path = scratch(name).join("app.adt");
    let db = Database::open(&path, &Log::new([SUM]), &options()).unwrap();
    runs(
        &db,
        r#"insert users { id: users.key(1), name: "n" }
           insert identities.EmailPass { id: identities.key(1), user: users.key(1), email: "e" }
           insert identities.Google { id: identities.key(2), user: users.key(1), gid: "g" }
           insert identities.Apple { id: identities.key(3), user: users.key(1), sub: "a" }"#,
    );
    let page = pick(&db);
    db.close().unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[page.0 as usize * 8192 + 300] ^= 0x10;
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn a_damaged_constructor_page_of_a_real_file_quarantines_its_constructor() {
    let path = damaged("quarantine_heap", |db| {
        let w = db.write_path();
        let heap = w.heap_named("app::identities", "Apple").unwrap();
        heap.pages(w.pager()).unwrap()[0]
    });
    let db = Database::open(&path, &Log::new([SUM]), &options()).unwrap();
    runs(&db, SIDES);
    assert_eq!("app::identities.Apple", quarantined(&db, ROWS));
    runs(&db, SIDES);
    assert_eq!("app::identities.Apple", quarantined(&db, ALL));
    db.close().unwrap();
    let db = Database::open(&path, &Log::new([SUM]), &options()).unwrap();
    runs(&db, SIDES);
    assert_eq!("app::identities.Apple", quarantined(&db, ALL));
    db.close().unwrap();
}

#[test]
fn a_damaged_index_page_of_a_real_file_quarantines_its_index() {
    let path = damaged("quarantine_index", |db| {
        let w = db.write_path();
        let index = w
            .index_named("app::identities", "index (user) in (Google | Apple)")
            .unwrap();
        *index.tree().pages(w.pager()).unwrap().last().unwrap()
    });
    let db = Database::open(&path, &Log::new([SUM]), &options()).unwrap();
    assert!(quarantined(&db, BY_USER).contains("(user)"));
    runs(&db, BY_USER);
    assert!(quarantined(&db, INSERT).contains("(user)"));
    db.close().unwrap();
    let db = Database::open(&path, &Log::new([SUM]), &options()).unwrap();
    runs(&db, BY_USER);
    assert!(quarantined(&db, INSERT).contains("(user)"));
    db.close().unwrap();
}
