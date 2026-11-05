//! Reads of the past in process over the simulator: the retention window setting, `at commit`
//! and `at instant` blocks, their refusals, vacuum moving versions to history, and a crash and
//! reopen with history kept.

use adtb_simkit::api;

use std::time::SystemTime;

use adtb::catalog::version_hash;
use adtb::runtime::Clock;
use adtb::{
    CallOptions, Database, Error, Log, OpenError, OpenOptions, Position, Registration,
    RejectReason, TxnCallOptions, TxnOptions, Value,
};
use adtb_lang::evolve::Source;
use adtb_lang::parse::{Options, parse_migration};
use adtb_lang::types::{Base, Ty};
use adtb_sim::{Config, Sim, SimRuntime};

use api::{options, run, sorted};

const STOCK: &str = r"module stock
add module stock
add type Item { id: Key items, name: Text }
  = Open   { qty: I64 }
  | Closed
add relation items : Item key id generate sequence
add type Order = { id: Key orders, item: Key items }
add relation orders : Order key id generate sequence
";

/// A migration after [`STOCK`] that rewrites `items`.
const RENAME: &str = r"module stock
rename field Item.Open.qty to count
";

const NAMES: &str = "from i in items select i.name";

fn open_with(rt: &SimRuntime, log: &Log) -> Database<SimRuntime> {
    Database::open_in(rt.clone(), "stock.adt", log, &options(2)).expect("the database opens")
}

fn open(rt: &SimRuntime) -> Database<SimRuntime> {
    open_with(rt, &Log::new([STOCK]))
}

/// Returns each row of the one result of `text`, sorted, or the rejection.
fn read(db: &Database<SimRuntime>, text: &str) -> Result<Vec<String>, Error> {
    let mut out = db.query_text(text, &[], &CallOptions::default())?;
    assert_eq!(1, out.len(), "`{text}` has one result");
    Ok(sorted(out.remove(0)))
}

fn rows(db: &Database<SimRuntime>, text: &str) -> Vec<String> {
    read(db, text).unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"))
}

fn run_text(db: &Database<SimRuntime>, text: &str) {
    db.query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
}

/// Commit `text` as one transaction. Returns its commit position.
fn commit(db: &Database<SimRuntime>, text: &str) -> Position {
    let mut t = db
        .begin(&TxnOptions::default())
        .expect("a transaction begins");
    t.query_text(text, &[], &TxnCallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
    t.commit(None).expect("the transaction commits")
}

/// Run a read-only query, whose safe snapshot is kept for reads of the past.
fn mark(db: &Database<SimRuntime>) {
    rows(db, "from i in items select key i");
}

fn keep(db: &Database<SimRuntime>, window: &str) {
    run_text(
        db,
        &format!("set setting RetentionWindow = {{ length: {window} }}"),
    );
}

fn at(p: Position) -> String {
    format!("Position {{ epoch: {}, lsn: {} }}", p.epoch, p.lsn)
}

fn past_names(p: Position) -> String {
    format!("at commit {} {{ {NAMES} }}", at(p))
}

/// Returns `pins` naming the version `migrations` make, with its hash.
fn pins(migrations: &[&str]) -> String {
    let mut source = Source::default();
    for text in migrations {
        let file = parse_migration(text, Options::default())
            .ok()
            .unwrap_or_else(|ds| panic!("`{text}` parses: {ds:?}"));
        source = source
            .migrate(&file, text)
            .unwrap_or_else(|r| panic!("`{text}` applies: {r:?}"));
    }
    let schema = source.schema().expect("the source builds");
    let hash: String = version_hash(&schema, &source)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("pins {} ({hash})", migrations.len())
}

fn names(ns: &[&str]) -> Vec<String> {
    ns.iter().map(|n| format!("{n:?}")).collect()
}

fn rejection(e: &Error) -> &RejectReason {
    match e {
        Error::Rejected { reason } => reason,
        other => panic!("not a rejection: {other:?}"),
    }
}

/// Returns the oldest position a `HistoryUnavailable` rejection names.
fn unavailable(e: &Error) -> Position {
    match rejection(e) {
        RejectReason::HistoryUnavailable { oldest } => *oldest,
        other => panic!("not HistoryUnavailable: {other:?}"),
    }
}

/// Open the stock database keeping an hour of history, with `ann` and `bo` committed at the
/// first position returned and `ann` renamed `cy` and `bo` closed at the second, a safe snapshot
/// kept after each.
fn history(rt: &SimRuntime) -> (Database<SimRuntime>, Position, Position) {
    let db = open(rt);
    keep(&db, "1h");
    let first = commit(
        &db,
        r#"insert items.Open { id: items.key(1), name: "ann", qty: 3 }
           insert items.Open { id: items.key(2), name: "bo", qty: 5 }"#,
    );
    mark(&db);
    let second = commit(
        &db,
        r#"update i in items.Open where key i == items.key(1) set Open { i with name: "cy" }
           delete i in items where key i == items.key(2)
           insert items.Closed { id: items.key(2), name: "bo" }"#,
    );
    mark(&db);
    (db, first, second)
}

fn with_history<T>(
    seed: u64,
    body: impl FnOnce(&SimRuntime, &Database<SimRuntime>, Position, Position) -> T,
) -> T {
    api::once(seed, |rt| {
        let (db, first, second) = history(&rt);
        body(&rt, &db, first, second)
    })
}

#[test]
fn a_quiescent_commit_survives_a_crash_before_any_history_read() {
    let mut sim = Sim::new(27, Config::default());
    let committed = run(&mut sim, |rt| {
        let db = open(&rt);
        keep(&db, "1h");
        let committed = commit(
            &db,
            r#"insert items.Closed { id: items.key(1), name: "ann" }"#,
        );
        std::mem::forget(db);
        committed
    });
    run(&mut sim, |rt| {
        let db = open(&rt);
        assert_eq!(names(&["ann"]), rows(&db, &past_names(committed)));
        assert_eq!(
            vec![format!("{{ lsn: {} }}", committed.lsn)],
            rows(
                &db,
                "from c in sys::commit_times select { lsn: c.position.lsn }"
            )
        );
    });
}

#[test]
fn a_read_at_an_instant_reads_the_latest_commit_by_then() {
    with_history(7, |rt, db, _, _| {
        let between = rt.wall();
        commit(
            db,
            r#"insert items.Open { id: items.key(3), name: "eve", qty: 1 }"#,
        );
        mark(db);
        let micros = between
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("after the epoch")
            .as_micros();
        let t = Value::Instant(i64::try_from(micros).expect("micros fit"));
        let mut out = db
            .query_text(
                &format!("at instant t {{ {NAMES} }}"),
                &[("t", Ty::Base(Base::Instant), t)],
                &CallOptions::default(),
            )
            .expect("the read runs");
        assert_eq!(names(&["bo", "cy"]), sorted(out.remove(0)));
    });
}

#[test]
fn a_read_before_a_migration_that_rewrote_what_it_reads_is_unavailable() {
    let mut sim = Sim::new(15, Config::default());
    let first = run(&mut sim, |rt| {
        let (db, first, _) = history(&rt);
        db.close().expect("close");
        first
    });
    run(&mut sim, |rt| {
        let db = open_with(&rt, &Log::new([STOCK, RENAME]));
        mark(&db);
        let e = read(&db, &past_names(first)).expect_err("the items heaps are new");
        unavailable(&e);
        let orders = format!("at commit {} {{ from o in orders select o }}", at(first));
        assert!(rows(&db, &orders).is_empty());
    });
}

#[test]
fn history_survives_a_crash_and_reopen() {
    let mut sim = Sim::new(20, Config::default());
    let (first, second) = run(&mut sim, |rt| {
        let (db, first, second) = history(&rt);
        run_text(&db, "vacuum items");
        commit(
            &db,
            "insert orders { id: orders.key(1), item: items.key(2) }",
        );
        std::mem::forget(db);
        (first, second)
    });
    run(&mut sim, |rt| {
        let db = open(&rt);
        assert_eq!(names(&["ann", "bo"]), rows(&db, &past_names(first)));
        assert_eq!(names(&["bo", "cy"]), rows(&db, &past_names(second)));
        commit(&db, "delete i in items where key i == items.key(1)");
        mark(&db);
        run_text(&db, "vacuum items");
        assert_eq!(names(&["ann", "bo"]), rows(&db, &past_names(first)));
        db.close().expect("close");
    });
    let report = run(&mut sim, |rt| {
        let options = adtb::storage::Options {
            page_size: 8192,
            threads: 4,
            ..adtb::storage::Options::new("stock.adt")
        };
        let (pager, _) = adtb::storage::Pager::open(rt.clone(), &options).expect("the pager");
        let (catalog, txns) = adtb::catalog::Catalog::open(rt, &pager).expect("the catalog");
        adtb::verify::verify(&pager, &catalog.inventory(&txns.commit_log()))
    });
    assert!(report.is_clean() && report.complete, "{report:#?}");
}

/// Returns bundle `reports-1.0`, pinned at the version `migrations` make, whose query
/// `names_then` reads the names of items at a position, its block pinned at the version
/// `block` makes.
fn reports(migrations: &[&str], block: &[&str]) -> Registration {
    let text = format!(
        "bundle reports-1.0 {} edition \"2026-10\"\nimport stock\nquery names_then (c: Position) -> Set Text =\n  at commit c {} {{ {NAMES} }}\n",
        pins(migrations),
        pins(block)
    );
    Registration::bundle(&text).unwrap_or_else(|e| panic!("`{text}` is a bundle file: {e:?}"))
}

#[test]
fn a_registered_block_pinned_past_the_databases_version_is_refused_at_open() {
    api::once(22, |rt| {
        let opts = OpenOptions {
            registrations: vec![reports(&[STOCK], &[STOCK, RENAME])],
            ..options(2)
        };
        match Database::open_in(rt, "stock.adt", &Log::new([STOCK]), &opts) {
            Err(OpenError::RegistrationRefused { bundle, .. }) => assert_eq!("reports-1.0", bundle),
            other => panic!("the registration is refused: {other:?}"),
        }
    });
}
