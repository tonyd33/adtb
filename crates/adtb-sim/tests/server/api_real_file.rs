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
fn a_created_file_is_locked_against_another_process() {
    let path = scratch("created-locked").join("shop.adt");
    let db = adtb::open(&path, &Log::new([SHOP]), &options()).expect("create");
    let other = std::fs::File::open(&path).expect("the created file");
    assert!(
        matches!(other.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
        "the name the create published is locked"
    );
    db.close().expect("close");
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
    let db = adtb::open(&path, &log, &options()).expect("create");
    db.query_text(
        r#"insert customers { name: "ann" }"#,
        &[],
        &CallOptions::default(),
    )
    .expect("the acknowledged row");
    db.close().expect("close");
    let refused = Log::new([SHOP, "module shop\ndrop relation missing"]);
    let got = adtb::open(&path, &refused, &options()).map(drop);
    let Err(OpenError::MigrationRefused { version, reason }) = got else {
        panic!("expected the invalid migration, found {got:?}");
    };
    assert_eq!(2, version);
    assert_eq!(
        adtb::Error::Rejected {
            reason: adtb::RejectReason::SchemaRefused {
                operator: String::new(),
                detail: "module `shop` declares no relation `missing`".into(),
            },
        },
        reason
    );
    assert!(
        path.is_file(),
        "an open that created nothing removes nothing"
    );
    assert!(path.with_file_name("shop.adt-log").is_dir());
    let db = adtb::open(&path, &log, &OpenOptions::default()).expect("reopen");
    assert_eq!(vec![r#""ann""#], names(&db));
}

/// Returns each entry under `dir`, by name, with the bytes of each file.
fn tree(dir: &Path) -> std::collections::BTreeMap<std::ffi::OsString, Vec<u8>> {
    std::fs::read_dir(dir)
        .expect("the directory")
        .map(|e| {
            let e = e.expect("an entry");
            (e.file_name(), std::fs::read(e.path()).expect("a file"))
        })
        .collect()
}

fn entries(dir: &Path) -> Vec<std::ffi::OsString> {
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .expect("the directory")
        .map(|e| e.expect("an entry").file_name())
        .collect();
    out.sort();
    out
}

fn corrupt_detail(got: &Result<(), OpenError>) -> &str {
    match got {
        Err(OpenError::Corrupt { detail }) => detail,
        other => panic!("expected a corrupt file, got {other:?}"),
    }
}

#[test]
fn a_create_refused_on_a_log_with_no_file_leaves_the_log_and_makes_no_file() {
    let dir = scratch("log-no-file");
    let (path, aside) = (dir.join("shop.adt"), dir.join("moved.adt"));
    let log_dir = dir.join("shop.adt-log");
    let log = Log::new([SHOP]);
    let db = adtb::open(&path, &log, &options()).expect("create");
    db.query_text(
        r#"insert customers { id: customers.key(1), name: "ann" }"#,
        &[],
        &CallOptions::default(),
    )
    .expect("insert");
    db.close().expect("close");
    std::fs::rename(&path, &aside).expect("move the file away");
    let before = tree(&log_dir);
    assert!(!before.is_empty(), "the close left a log");

    let got = adtb::open(&path, &log, &options()).map(drop);
    assert!(
        corrupt_detail(&got) == "the log directory holds a log that no file here wrote",
        "the refusal names the log: {got:?}"
    );
    assert!(
        !path.exists(),
        "a refused create leaves no file at the path"
    );
    assert_eq!(
        before,
        tree(&log_dir),
        "a refused create leaves the log whole"
    );

    std::fs::rename(&aside, &path).expect("move the file back");
    let db = adtb::open(&path, &log, &OpenOptions::default()).expect("the retry opens it");
    assert_eq!(vec![r#""ann""#], names(&db));
}

#[test]
fn a_create_refused_on_a_log_of_only_an_unknown_file_leaves_it() {
    let dir = scratch("log-unknown");
    let path = dir.join("shop.adt");
    let log_dir = dir.join("shop.adt-log");
    std::fs::create_dir(&log_dir).expect("the log directory");
    std::fs::write(log_dir.join("mine"), b"kept").expect("a file");
    let before = tree(&log_dir);

    let got = adtb::open(&path, &Log::new([SHOP]), &options()).map(drop);
    assert!(
        corrupt_detail(&got) == "the log directory holds a log that no file here wrote",
        "the refusal names the log: {got:?}"
    );
    assert!(
        !path.exists(),
        "a refused create leaves no file at the path"
    );
    assert_eq!(before, tree(&log_dir));
}

#[test]
fn a_refused_create_on_a_file_it_found_empty_keeps_it_empty_and_makes_no_log() {
    let dir = scratch("refused-empty");
    let path = dir.join("shop.adt");
    std::fs::write(&path, b"").expect("an empty file");
    let refused = Log::new([SHOP, "module shop\nremove relation missing"]);
    let got = adtb::open(&path, &refused, &options()).map(drop);
    assert!(
        matches!(got, Err(OpenError::MigrationRefused { version: 2, .. })),
        "{got:?}"
    );
    assert_eq!(
        vec![std::ffi::OsString::from("shop.adt")],
        entries(&dir),
        "the file was not this call's, and nothing was published over it"
    );
    assert_eq!(Vec::<u8>::new(), std::fs::read(&path).expect("the file"));
    adtb::open(&path, &Log::new([SHOP]), &options()).expect("a later open succeeds");
}

/// Stamp the header copy at offset `at` of `file` as copy `copy` of layout `layout`, whole.
fn stamp_layout(file: &mut [u8], at: usize, copy: u64, layout: u32) {
    const REGION: usize = 4096;
    let region = &mut file[at..at + REGION];
    region[12..16].copy_from_slice(&layout.to_le_bytes());
    region[8..12].fill(0);
    let sum = adtb::storage::Crc32c::new()
        .update(&copy.to_le_bytes())
        .update(region)
        .finish();
    region[8..12].copy_from_slice(&sum.to_le_bytes());
}

#[test]
fn a_file_of_another_layout_is_refused_naming_the_layout_and_kept() {
    let dir = scratch("old-layout");
    let path = dir.join("shop.adt");
    let log = Log::new([SHOP]);
    adtb::open(&path, &log, &options())
        .expect("create")
        .close()
        .expect("close");
    let mut bytes = std::fs::read(&path).expect("the file");
    let page_size = u32::from_le_bytes(bytes[40..44].try_into().expect("four bytes")) as usize;
    stamp_layout(&mut bytes, 0, 0, 2);
    stamp_layout(&mut bytes, page_size, 1, 2);
    std::fs::write(&path, &bytes).expect("the old layout");

    for opening in [options(), OpenOptions::default()] {
        let got = adtb::open(&path, &log, &opening).map(drop);
        assert!(
            corrupt_detail(&got) == "the file header has layout 2, not one this engine reads",
            "the refusal names the layout: {got:?}"
        );
        assert_eq!(bytes, std::fs::read(&path).expect("the file"));
    }
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
#[expect(clippy::disallowed_methods, reason = "until adtb::testing covers it")]
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
#[expect(clippy::disallowed_methods, reason = "until adtb::testing covers it")]
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

#[test]
#[expect(clippy::disallowed_methods, reason = "until adtb::testing covers it")]
fn open_returns_on_a_real_file_cut_before_the_first_roles_page() {
    let path = scratch("cut").join("shop.adt");
    let log = Log::new([SHOP]);
    let at = {
        let db = adtb::open(&path, &log, &options()).expect("create");
        let w = db.write_path();
        let page = w
            .heap_named("catalog::roles", "Role")
            .unwrap()
            .pages(w.pager())
            .unwrap()[0];
        let at = page.0 * w.pager().page_size() as u64;
        db.close().expect("a close");
        at
    };
    std::fs::File::options()
        .write(true)
        .open(&path)
        .expect("the file")
        .set_len(at)
        .expect("the cut");
    let db = adtb::open(&path, &log, &options()).expect("an open for verify");
    let said = db.query_text(
        "from c in customers select c.name",
        &[],
        &CallOptions::default(),
    );
    assert!(
        matches!(
            said,
            Err(adtb::Error::Fatal {
                reason: adtb::FatalReason::Quarantined { .. },
            })
        ),
        "{said:?}"
    );
    db.close().expect("a close");
}

#[test]
fn a_backup_of_a_real_file_restores_verifies_clean_and_continues() {
    let dir = scratch("backup");
    let (file, target, copy) = (
        dir.join("app.adt"),
        dir.join("nested/bk"),
        dir.join("copy.adt"),
    );
    let log = Log::new([SHOP]);
    let db = Database::open(&file, &log, &options()).unwrap();
    for (id, name) in [(1, "ann"), (2, "bob")] {
        let text = format!(r#"insert customers {{ id: customers.key({id}), name: "{name}" }}"#);
        db.query_text(&text, &[], &CallOptions::default()).unwrap();
    }
    let text = format!("backup to {:?}", target.display().to_string());
    db.query_text(&text, &[], &CallOptions::default()).unwrap();
    db.close().unwrap();

    let archive = adtb::ArchiveTarget::path(&target).unwrap();
    let opening = OpenOptions {
        create: None,
        ..options()
    };
    adtb::restore(&archive, &copy, adtb::RestorePoint::Latest, opening.clone()).unwrap();
    assert_eq!(
        adtb::Proof::Proven,
        adtb::prove(&archive, &log, &opening).unwrap()
    );
    assert_eq!(0, adtb::verify_file(&copy, &log, &opening).unwrap().count());

    let db = Database::open(&copy, &log, &opening).unwrap();
    assert_eq!(2, names(&db).len());
    db.query_text(
        r#"insert customers { id: customers.key(3), name: "cam" }"#,
        &[],
        &CallOptions::default(),
    )
    .unwrap();
    assert_eq!(3, names(&db).len());
    db.close().unwrap();
}

#[test]
fn backup_and_restore_write_through_no_symbolic_link() {
    use std::os::unix::fs::symlink;
    let dir = scratch("backup_links");
    let file = dir.join("app.adt");
    let log = Log::new([SHOP]);
    let db = Database::open(&file, &log, &options()).unwrap();
    db.query_text(
        r#"insert customers { id: customers.key(1), name: "ann" }"#,
        &[],
        &CallOptions::default(),
    )
    .unwrap();
    let back_up = |target: &Path| {
        let text = format!("backup to {:?}", target.display().to_string());
        db.query_text(&text, &[], &CallOptions::default())
            .map(|_| ())
    };
    let victim = dir.join("victim");
    std::fs::write(&victim, b"keep").unwrap();
    let elsewhere = dir.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    for (name, to) in [
        ("manifest", &victim),
        ("pages", &elsewhere),
        ("log", &elsewhere),
    ] {
        let target = dir.join(format!("link-{name}"));
        std::fs::create_dir(&target).unwrap();
        symlink(to, target.join(name)).unwrap();
        let got = back_up(&target);
        assert!(
            got.as_ref()
                .is_err_and(|e| e.to_string().contains("holds files")),
            "{name}: {got:?}"
        );
    }
    let target = dir.join("bk");
    back_up(&target).unwrap();
    db.close().unwrap();
    assert_eq!(b"keep".as_slice(), std::fs::read(&victim).unwrap());
    assert_eq!(0, std::fs::read_dir(&elsewhere).unwrap().count());

    let archive = adtb::ArchiveTarget::path(&target).unwrap();
    let opening = OpenOptions {
        create: None,
        ..options()
    };
    let restore_to =
        |path: &Path| adtb::restore(&archive, path, adtb::RestorePoint::Latest, opening.clone());
    let dangling = dir.join("dangling.adt");
    symlink(dir.join("absent.adt"), &dangling).unwrap();
    let got = restore_to(&dangling);
    assert!(
        got.as_ref()
            .is_err_and(|e| e.to_string().contains("exists")),
        "{got:?}"
    );
    assert!(!dir.join("absent.adt").exists());
    let linked = dir.join("linked.adt");
    symlink(&victim, &linked).unwrap();
    let got = restore_to(&linked);
    assert!(
        got.as_ref()
            .is_err_and(|e| e.to_string().contains("exists")),
        "{got:?}"
    );
    assert_eq!(b"keep".as_slice(), std::fs::read(&victim).unwrap());
    restore_to(&dir.join("copy.adt")).unwrap();
}

/// Answer a file or a dangling symbolic link at a target or a restore's log directory as occupied,
/// and back up under an ancestor that can be searched and not listed by others.
#[test]
fn a_file_or_dangling_link_at_a_target_is_occupied() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = scratch("backup_taken");
    let log = Log::new([SHOP]);
    let db = Database::open(dir.join("app.adt"), &log, &options()).unwrap();
    let back_up = |target: &Path| {
        let text = format!("backup to {:?}", target.display().to_string());
        db.query_text(&text, &[], &CallOptions::default())
            .map(|_| ())
    };
    let file = dir.join("file");
    std::fs::write(&file, b"keep").unwrap();
    let dangling = dir.join("dangling");
    symlink(dir.join("absent"), &dangling).unwrap();
    for target in [&file, &dangling] {
        let got = back_up(target);
        assert!(
            got.as_ref()
                .is_err_and(|e| e.to_string().contains("holds files")),
            "{target:?}: {got:?}"
        );
    }
    assert_eq!(b"keep".as_slice(), std::fs::read(&file).unwrap());
    assert!(!dir.join("absent").exists());

    let searched = dir.join("searched");
    std::fs::create_dir(&searched).unwrap();
    std::fs::set_permissions(&searched, std::fs::Permissions::from_mode(0o711)).unwrap();
    let target = searched.join("under/bk");
    back_up(&target).unwrap();
    db.close().unwrap();

    let archive = adtb::ArchiveTarget::path(&target).unwrap();
    let opening = OpenOptions {
        create: None,
        ..options()
    };
    let restore_to =
        |path: &Path| adtb::restore(&archive, path, adtb::RestorePoint::Latest, opening.clone());
    std::fs::write(dir.join("one.adt-log"), b"keep").unwrap();
    symlink(dir.join("absent"), dir.join("two.adt-log")).unwrap();
    for name in ["one.adt", "two.adt"] {
        let got = restore_to(&dir.join(name));
        assert!(
            got.as_ref()
                .is_err_and(|e| e.to_string().contains("exists")),
            "{name}: {got:?}"
        );
        assert!(!dir.join(name).exists(), "{name}");
    }
    restore_to(&dir.join("copy.adt")).unwrap();
}
