//! Reads of the past in process over the simulator: the retention window setting, `at commit`
//! and `at instant` blocks, their refusals, vacuum moving versions to history, and a crash and
//! reopen with history kept.

#[expect(dead_code, reason = "this binary uses part of the API helpers")]
#[path = "support/api.rs"]
mod api;

use std::time::{Duration, SystemTime};

use adtb::catalog::version_hash;
use adtb::runtime::Clock;
use adtb::{
    CallOptions, Called, Database, Error, Log, OpenError, OpenOptions, Position, Registration,
    RejectReason, TxnCallOptions, TxnOptions, Value,
};
use adtb_lang::evolve::Source;
use adtb_lang::parse::{Options, parse_migration};
use adtb_lang::types::{Base, Ty};
use adtb_sim::{Config, Sim, SimRuntime};

use api::{options, printed, run, sorted};

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

/// A migration after [`RENAME`] that undoes it.
const RENAME_BACK: &str = r"module stock
rename field Item.Open.count to qty
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

/// Commits `text` as one transaction. Returns its commit position.
fn commit(db: &Database<SimRuntime>, text: &str) -> Position {
    let mut t = db
        .begin(&TxnOptions::default())
        .expect("a transaction begins");
    t.query_text(text, &[], &TxnCallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
    t.commit(None).expect("the transaction commits")
}

/// Runs a read-only query, whose safe snapshot is kept for reads of the past.
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

/// Opens the stock database keeping an hour of history, with `ann` and `bo` committed at the
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
fn a_just_committed_position_is_readable_without_an_intervening_read() {
    api::once(24, |rt| {
        let db = open(&rt);
        keep(&db, "1h");
        let first = commit(
            &db,
            r#"insert items.Closed { id: items.key(1), name: "ann" }"#,
        );
        assert_eq!(names(&["ann"]), rows(&db, &past_names(first)));
        let second = commit(
            &db,
            r#"insert items.Closed { id: items.key(2), name: "bo" }"#,
        );
        db.close().expect("close");
        let db = open(&rt);
        assert_eq!(names(&["ann"]), rows(&db, &past_names(first)));
        assert_eq!(names(&["ann", "bo"]), rows(&db, &past_names(second)));
    });
}

#[test]
fn commit_times_and_safe_snapshots_describe_each_quiescent_commit() {
    api::once(25, |rt| {
        let db = open(&rt);
        keep(&db, "1h");
        let first = commit(
            &db,
            r#"insert items.Closed { id: items.key(1), name: "ann" }"#,
        );
        let second = commit(
            &db,
            r#"insert items.Closed { id: items.key(2), name: "bo" }"#,
        );
        assert_eq!(
            vec![
                format!("{{ lsn: {} }}", first.lsn),
                format!("{{ lsn: {} }}", second.lsn)
            ],
            rows(
                &db,
                "from c in sys::commit_times select { lsn: c.position.lsn }"
            )
        );
        assert_eq!(
            rows(&db, "from c in sys::commit_times select c.position"),
            rows(&db, "from s in sys::safe_snapshots select s.position")
        );
        let q = "from c in sys::commit_times, n in (at instant c.at { from i in items select i.name }) select { lsn: c.position.lsn, name: n }";
        let expected = rows(&db, q);
        assert_eq!(
            vec![
                format!("{{ lsn: {}, name: \"ann\" }}", first.lsn),
                format!("{{ lsn: {}, name: \"ann\" }}", second.lsn),
                format!("{{ lsn: {}, name: \"bo\" }}", second.lsn),
            ],
            expected
        );
        db.close().expect("close");
        let db = open(&rt);
        assert_eq!(expected, rows(&db, q));
    });
}

#[test]
fn rolled_back_and_failed_writes_publish_no_commit_time_or_marker() {
    api::once(26, |rt| {
        let db = open(&rt);
        keep(&db, "1h");
        let first = commit(
            &db,
            r#"insert items.Closed { id: items.key(1), name: "ann" }"#,
        );
        let commits = rows(&db, "from c in sys::commit_times select c");
        let markers = rows(&db, "from s in sys::safe_snapshots select s");
        let mut tx = db.begin(&TxnOptions::default()).expect("begin");
        tx.query_text(
            r#"insert items.Closed { id: items.key(2), name: "bo" }"#,
            &[],
            &TxnCallOptions::default(),
        )
        .expect("write");
        tx.rollback().expect("rollback");
        let failed = read(
            &db,
            r#"insert items.Closed { id: items.key(1), name: "duplicate" }"#,
        )
        .expect_err("the duplicate key is refused");
        assert!(
            matches!(rejection(&failed), RejectReason::KeyViolation { .. }),
            "{failed:?}"
        );
        assert_eq!(commits, rows(&db, "from c in sys::commit_times select c"));
        assert_eq!(markers, rows(&db, "from s in sys::safe_snapshots select s"));
        assert_eq!(names(&["ann"]), rows(&db, &past_names(first)));
        db.close().expect("close");
        let db = open(&rt);
        assert_eq!(commits, rows(&db, "from c in sys::commit_times select c"));
        assert_eq!(markers, rows(&db, "from s in sys::safe_snapshots select s"));
    });
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
fn a_commit_with_another_writer_running_does_not_publish_an_unsafe_marker() {
    api::once(28, |rt| {
        let db = open(&rt);
        keep(&db, "1h");
        let first = commit(
            &db,
            r#"insert items.Closed { id: items.key(1), name: "ann" }"#,
        );
        let mut before = db.begin(&TxnOptions::default()).expect("begin");
        let second = commit(
            &db,
            r#"insert items.Closed { id: items.key(2), name: "bo" }"#,
        );
        assert_eq!(names(&["ann"]), rows(&db, &past_names(second)));
        let err = before
            .query_text(&past_names(second), &[], &TxnCallOptions::default())
            .expect_err("the commit follows the reader's snapshot");
        unavailable(&err);
        assert_eq!(
            vec![format!("{{ lsn: {} }}", first.lsn)],
            rows(
                &db,
                "from s in sys::safe_snapshots select { lsn: s.position.lsn }"
            )
        );
        before.rollback().expect("rollback");
    });
}

#[test]
fn the_retention_window_is_a_file_setting_zero_by_default() {
    api::once(0, |rt| {
        let db = open(&rt);
        let first = commit(
            &db,
            r#"insert items.Closed { id: items.key(1), name: "ann" }"#,
        );
        mark(&db);
        unavailable(&read(&db, &past_names(first)).expect_err("no history is kept"));
        keep(&db, "2h");
        let length = "from s in catalog::settings select match s
                        | RetentionWindow w -> w.length
                        | _ -> 0s";
        assert!(
            rows(&db, length).contains(&"2h".to_owned()),
            "{:?}",
            rows(&db, length)
        );
        let second = commit(
            &db,
            r#"insert items.Closed { id: items.key(2), name: "bo" }"#,
        );
        mark(&db);
        commit(&db, "delete i in items where key i == items.key(2)");
        mark(&db);
        assert_eq!(names(&["ann", "bo"]), rows(&db, &past_names(second)));
    });
}

#[test]
fn a_read_at_a_commit_sees_the_rows_committed_by_then() {
    with_history(1, |_, db, first, second| {
        assert_eq!(names(&["ann", "bo"]), rows(db, &past_names(first)));
        assert_eq!(names(&["bo", "cy"]), rows(db, &past_names(second)));
        assert_eq!(names(&["bo", "cy"]), rows(db, NAMES));
    });
}

#[test]
fn a_read_of_a_constructor_relation_at_a_commit_reads_its_rows_then() {
    with_history(2, |_, db, first, _| {
        let open = format!(
            "at commit {} {{ from i in items.Open select i.name }}",
            at(first)
        );
        assert_eq!(names(&["ann", "bo"]), rows(db, &open));
        let closed = format!(
            "at commit {} {{ from i in items.Closed select i.name }}",
            at(first)
        );
        assert!(rows(db, &closed).is_empty());
    });
}

#[test]
fn a_position_between_safe_snapshots_reads_the_latest_one_before_it() {
    with_history(3, |_, db, first, second| {
        let between = Position {
            epoch: second.epoch,
            lsn: second.lsn - 1,
        };
        assert!(first < between, "{first:?} {between:?}");
        assert_eq!(names(&["ann", "bo"]), rows(db, &past_names(between)));
    });
}

#[test]
fn an_at_block_reads_beside_the_current_state_in_one_query() {
    with_history(4, |_, db, first, _| {
        let q = format!(
            "from p in (at commit {} {{ from i in items.Open select i }}), q in items.Closed
             where key p == key q
             select {{ item: q.name, was: p.qty }}",
            at(first)
        );
        assert_eq!(vec![r#"{ item: "bo", was: 5 }"#], rows(db, &q));
    });
}

#[test]
fn a_key_lookup_in_an_at_block_reads_the_row_then() {
    with_history(5, |_, db, first, _| {
        let q = format!(
            "at commit {} {{ from i in items where key i == items.key(1) select i.name }}",
            at(first)
        );
        assert_eq!(names(&["ann"]), rows(db, &q));
    });
}

#[test]
fn a_reference_followed_in_an_at_block_reads_its_target_then() {
    with_history(6, |_, db, _, _| {
        let placed = commit(
            db,
            "insert orders { id: orders.key(1), item: items.key(1) }",
        );
        mark(db);
        commit(
            db,
            r#"update i in items.Open where key i == items.key(1) set Open { i with name: "dee" }"#,
        );
        mark(db);
        let q = format!(
            "at commit {} {{ from o in orders select o.item.name }}",
            at(placed)
        );
        assert_eq!(names(&["cy"]), rows(db, &q));
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
fn a_position_after_the_readers_snapshot_is_unavailable() {
    with_history(8, |_, db, _, second| {
        let later = Position {
            epoch: second.epoch,
            lsn: second.lsn + 1_000_000,
        };
        let e = read(db, &past_names(later)).expect_err("a later position is refused");
        let oldest = unavailable(&e);
        assert!(oldest <= second, "{oldest:?}");
    });
}

#[test]
fn without_a_retention_window_every_read_of_the_past_is_unavailable() {
    api::once(9, |rt| {
        let db = open(&rt);
        let first = commit(
            &db,
            r#"insert items.Closed { id: items.key(1), name: "ann" }"#,
        );
        mark(&db);
        let e = read(&db, &past_names(first)).expect_err("no history is kept");
        unavailable(&e);
    });
}

#[test]
fn an_at_block_is_evaluated_only_where_its_guards_hold() {
    api::once(10, |rt| {
        let db = open(&rt);
        commit(
            &db,
            r#"insert items.Closed { id: items.key(1), name: "ann" }"#,
        );
        let q = "from i in items
                 where false
                 select count(j for j in (at commit Position { epoch: 0, lsn: 0 } { from k in items select k }))";
        assert!(rows(&db, q).is_empty());
    });
}

#[test]
fn an_at_block_whose_body_is_not_a_set_is_a_type_error() {
    api::once(11, |rt| {
        let db = open(&rt);
        let e = read(&db, "at commit Position { epoch: 0, lsn: 0 } { 1 }")
            .expect_err("the body is not a set");
        assert!(
            matches!(rejection(&e), RejectReason::Invalid { .. }),
            "{e:?}"
        );
    });
}

#[test]
fn vacuum_keeps_what_the_window_keeps_and_reads_of_the_past_still_see_it() {
    with_history(12, |_, db, first, second| {
        run_text(db, "vacuum items");
        assert_eq!(names(&["ann", "bo"]), rows(db, &past_names(first)));
        assert_eq!(names(&["bo", "cy"]), rows(db, &past_names(second)));
        let q = format!(
            "at commit {} {{ from i in items where key i == items.key(2) select i.name }}",
            at(first)
        );
        assert_eq!(names(&["bo"]), rows(db, &q));
        run_text(db, "vacuum items");
        assert_eq!(names(&["ann", "bo"]), rows(db, &past_names(first)));
    });
}

#[test]
fn past_the_window_a_read_is_unavailable_and_names_the_oldest_position_kept() {
    api::once(13, |rt| {
        let db = open(&rt);
        keep(&db, "50ms");
        let first = commit(
            &db,
            r#"insert items.Closed { id: items.key(1), name: "ann" }"#,
        );
        mark(&db);
        let until = rt.wall() + Duration::from_millis(200);
        let mut n = 2;
        while rt.wall() <= until {
            commit(
                &db,
                &format!(r#"insert items.Closed {{ id: items.key({n}), name: "x" }}"#),
            );
            n += 1;
        }
        let last = commit(&db, "delete i in items where key i == items.key(1)");
        mark(&db);
        run_text(&db, "vacuum items");
        let e = read(&db, &past_names(first)).expect_err("the first commit is outside the window");
        let oldest = unavailable(&e);
        assert!(
            first < oldest && oldest <= last,
            "{first:?} {oldest:?} {last:?}"
        );
        assert!(
            rows(
                &db,
                &format!(
                    "from c in sys::commit_times where c.position.lsn == {} select c",
                    first.lsn
                )
            )
            .is_empty()
        );
        assert!(
            rows(
                &db,
                &format!(
                    "from s in sys::safe_snapshots where s.position.lsn == {} select s",
                    first.lsn
                )
            )
            .is_empty()
        );
        let kept = rows(&db, &past_names(last));
        assert!(kept.contains(&"\"x\"".to_owned()), "{kept:?}");
    });
}

#[test]
fn shrinking_the_window_to_zero_makes_earlier_states_unavailable() {
    with_history(14, |_, db, first, _| {
        keep(db, "0s");
        assert!(rows(db, "from c in sys::commit_times select c").is_empty());
        assert!(rows(db, "from s in sys::safe_snapshots select s").is_empty());
        let e = read(db, &past_names(first)).expect_err("no state is kept");
        unavailable(&e);
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
fn a_block_pinned_at_a_version_that_stores_its_reads_otherwise_is_the_wrong_version() {
    let mut sim = Sim::new(16, Config::default());
    let first = run(&mut sim, |rt| {
        let (db, first, _) = history(&rt);
        db.close().expect("close");
        first
    });
    run(&mut sim, |rt| {
        let db = open_with(&rt, &Log::new([STOCK, RENAME]));
        mark(&db);
        let q = format!(
            "at commit {} {} {{ {NAMES} }}",
            at(first),
            pins(&[STOCK, RENAME])
        );
        let e = read(&db, &q).expect_err("version 1 stores items otherwise than version 2");
        match rejection(&e) {
            RejectReason::WrongVersion { version } => assert_eq!(1, *version),
            other => panic!("not WrongVersion: {other:?}"),
        }
    });
}

#[test]
fn a_block_pinned_past_the_databases_version_is_refused() {
    with_history(17, |_, db, first, _| {
        let later = [STOCK, RENAME, RENAME_BACK];
        let q = format!("at commit {} {} {{ {NAMES} }}", at(first), pins(&later));
        read(db, &q).expect_err("the database is at version 1");
    });
}

#[test]
fn a_block_pinned_at_the_version_of_its_state_reads_it() {
    with_history(18, |_, db, first, _| {
        let q = format!("at commit {} {} {{ {NAMES} }}", at(first), pins(&[STOCK]));
        assert_eq!(names(&["ann", "bo"]), rows(db, &q));
    });
}

#[test]
fn explain_shows_a_read_of_the_past() {
    with_history(19, |_, db, first, _| {
        let mut out = db
            .query_text(
                &format!("explain {}", past_names(first)),
                &[],
                &CallOptions::default(),
            )
            .expect("explain runs");
        let text = printed(out.remove(0)).join("\n");
        assert!(text.contains("Read the past"), "{text}");
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

fn position(p: Position) -> Value {
    Value::ctor(
        0,
        vec![
            Value::Int(i64::from(p.epoch)),
            Value::Int(i64::try_from(p.lsn).expect("an LSN fits")),
        ],
    )
}

#[test]
fn a_registered_query_reads_the_past_at_the_version_its_block_pins() {
    api::once(21, |rt| {
        let opts = OpenOptions {
            registrations: vec![reports(&[STOCK], &[STOCK])],
            ..options(2)
        };
        let db = Database::open_in(rt, "stock.adt", &Log::new([STOCK]), &opts)
            .expect("the database opens");
        keep(&db, "1h");
        let first = commit(
            &db,
            r#"insert items.Closed { id: items.key(1), name: "ann" }"#,
        );
        mark(&db);
        commit(&db, "delete i in items where key i == items.key(1)");
        mark(&db);
        let got = db
            .registered("reports-1.0")
            .expect("the bundle binds")
            .call(1, &[position(first)], &CallOptions::default())
            .expect("the call runs");
        match got {
            Called::Rows(r) => assert_eq!(names(&["ann"]), sorted(r)),
            other => panic!("a query returns rows: {other:?}"),
        }
    });
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
