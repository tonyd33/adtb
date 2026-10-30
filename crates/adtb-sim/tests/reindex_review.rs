#![expect(
    clippy::disallowed_methods,
    reason = "drives real threads and clocks against a real file; removing it needs these races on the simulator"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant, SystemTime};

use adtb::runtime::{Clock, Threads};
use adtb::storage::{Damage, Quarantine};
use adtb::{CallOptions, CancelToken, Database, Log, TxnCallOptions, TxnOptions};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_simkit::{api, crash};

const APP: &str = r"module app
add module app
add type Thing { id: Key things, owner: I64, note: Text } = A { a: Text } | B { b: Text }
add relation things : Thing key id generate sequence
  unique (a) in A
  index (owner) in (A | B)
add type Shift = { id: Key shifts, duty: I64 }
add relation shifts : Shift key id generate sequence
  index (duty)
";

fn path(name: &str) -> String {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("reindex_review")
        .join(name);
    let _gone = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.join("app.adt").to_str().unwrap().to_owned()
}

fn open(p: &str) -> Database {
    Database::open(p, &Log::new([APP]), &api::options(4)).unwrap()
}

fn run(db: &Database, t: &str) -> Result<Vec<String>, adtb::Error> {
    let mut r = db.query_text(t, &[], &CallOptions::default())?;
    Ok(r.pop().map(api::printed).unwrap_or_default())
}

fn idle(db: &Database) {
    while !run(
        db,
        r#"from j in sys::jobs where j.target != "" select j.id"#,
    )
    .unwrap()
    .is_empty()
    {
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[derive(Clone)]
struct Row {
    a: bool,
    owner: i64,
    val: String,
}

struct Writer {
    db: Database,
    model: BTreeMap<i64, Row>,
    next: i64,
    owner_base: i64,
    bad: Arc<Mutex<Vec<String>>>,
}

impl Writer {
    /// Record a failure of `what` unless it is a serialization failure.
    fn check(&self, e: &adtb::Error, what: &str) {
        let s = format!("{e:?}");
        if !s.contains("Serialization") {
            self.bad.lock().unwrap().push(format!("{what}: {s}"));
        }
    }

    fn insert(&mut self) {
        self.next += 1;
        let next = self.next;
        let owner = self.owner_base;
        let val = format!("{}{next}", "L".repeat(if next % 5 == 0 { 1000 } else { 1 }));
        let t = format!(
            r#"insert things.A {{ id: things.key({next}), owner: {owner}, note: "n", a: "{val}" }}"#
        );
        match run(&self.db, &t) {
            Ok(_) => {
                self.model.insert(
                    next,
                    Row {
                        a: true,
                        owner,
                        val,
                    },
                );
            }
            Err(e) => self.check(&e, "insert"),
        }
    }

    fn bulk_update(&mut self, round: i64) {
        let from = self.owner_base + (round % 3);
        let to = self.owner_base + ((round + 1) % 3);
        let t =
            format!("update t in things.A where t.owner == {from} set A {{ t with owner: {to} }}");
        match run(&self.db, &t) {
            Ok(_) => {
                for r in self.model.values_mut() {
                    if r.a && r.owner == from {
                        r.owner = to;
                    }
                }
            }
            Err(e) => self.check(&e, "bulk update"),
        }
    }

    fn heap_only_update(&self, k: i64, round: i64) {
        let c = if self.model[&k].a { "A" } else { "B" };
        let big = if round % 7 == 0 {
            "N".repeat(20_000)
        } else {
            format!("h{round}")
        };
        let t = format!(
            r#"update t in things.{c} where key t == things.key({k}) set {c} {{ t with note: "{big}" }}"#
        );
        if let Err(e) = run(&self.db, &t) {
            self.check(&e, "hot");
        }
    }

    fn retag(&mut self, k: i64) {
        let was_a = self.model[&k].a;
        let t = if was_a {
            format!(
                r#"update t in things.A where key t == things.key({k}) set B {{ t with b: "b{k}" }}"#
            )
        } else {
            format!(
                r#"update t in things.B where key t == things.key({k}) set A {{ t with a: "a{k}" }}"#
            )
        };
        match run(&self.db, &t) {
            Ok(_) => {
                let m = self.model.get_mut(&k).unwrap();
                m.a = !was_a;
                m.val = if was_a {
                    format!("b{k}")
                } else {
                    format!("a{k}")
                };
            }
            Err(e) => self.check(&e, "retag"),
        }
    }

    fn delete(&mut self, k: i64) {
        let c = if self.model[&k].a { "A" } else { "B" };
        let t = format!("delete t in things.{c} where key t == things.key({k})");
        match run(&self.db, &t) {
            Ok(_) => {
                self.model.remove(&k);
            }
            Err(e) => self.check(&e, "delete"),
        }
    }

    fn refused_duplicate(&mut self) {
        if let Some(r) = self.model.values().find(|r| r.a) {
            self.next += 1;
            let t = format!(
                r#"insert things.A {{ id: things.key({}), owner: 0, note: "n", a: "{}" }}"#,
                self.next, r.val
            );
            if run(&self.db, &t).is_ok() {
                self.bad
                    .lock()
                    .unwrap()
                    .push(format!("duplicate a {} admitted", r.val));
            }
        }
    }

    fn rollback(&mut self) {
        self.next += 1;
        let mut t = self.db.begin(&TxnOptions::default()).unwrap();
        let i = format!(
            r#"insert things.B {{ id: things.key({}), owner: {}, note: "n", b: "gone" }}"#,
            self.next, self.owner_base
        );
        let _refused = t.query_text(&i, &[], &TxnCallOptions::default());
        t.rollback().unwrap();
    }

    fn read_by_owner(&self, round: i64) {
        let o = self.owner_base + round % 3;
        let want = self.model.values().filter(|r| r.owner == o).count();
        match run(
            &self.db,
            &format!("from t in things where t.owner == {o} select t.id"),
        ) {
            Ok(got) if got.len() != want => {
                let what = format!("owner {o}: index gives {} rows, model {want}", got.len());
                self.bad.lock().unwrap().push(what);
            }
            Ok(_) => {}
            Err(e) => self.check(&e, "read"),
        }
    }

    fn seed(&mut self) {
        for owner in 0..3 {
            for _ in 0..4 {
                self.next += 1;
                let (next, owner) = (self.next, self.owner_base + owner);
                let val = format!("a{next}");
                let t = format!(
                    r#"insert things.A {{ id: things.key({next}), owner: {owner}, note: "n", a: "{val}" }}"#
                );
                run(&self.db, &t).unwrap();
                self.model.insert(
                    next,
                    Row {
                        a: true,
                        owner,
                        val,
                    },
                );
            }
        }
    }
}

fn writer(
    db: Database,
    w: i64,
    stop: Arc<AtomicBool>,
    bad: Arc<Mutex<Vec<String>>>,
) -> BTreeMap<i64, Row> {
    let mut me = Writer {
        db,
        model: BTreeMap::new(),
        next: w * 1_000_000,
        owner_base: w * 100,
        bad,
    };
    let mut x = (w as u64 + 1) * 0x9E37_79B9;
    me.seed();
    let mut round = 0i64;
    while !stop.load(Ordering::Acquire) {
        round += 1;
        x = x.wrapping_mul(0x5851_F42D_4C95_7F2D).wrapping_add(11);
        let keys: Vec<i64> = me.model.keys().copied().collect();
        let k = if keys.is_empty() {
            0
        } else {
            keys[(x >> 20) as usize % keys.len()]
        };
        let present = me.model.contains_key(&k);
        match (x >> 40) % 8 {
            0 => me.insert(),
            1 | 2 => me.bulk_update(round),
            3 if present => me.heap_only_update(k, round),
            4 if present => me.retag(k),
            5 if me.model.len() > 8 && present => me.delete(k),
            6 => me.refused_duplicate(),
            _ => me.rollback(),
        }
        if round % 5 == 0 {
            me.read_by_owner(round);
        }
    }
    me.model
}

fn stress(name: &str, reindexes: usize, verify_beside: bool) {
    let p = path(name);
    let db = open(&p);
    let stop = Arc::new(AtomicBool::new(false));
    let bad = Arc::new(Mutex::new(Vec::new()));
    let ws: Vec<_> = (1..=3)
        .map(|w| {
            let (db, stop, bad) = (db.clone(), stop.clone(), bad.clone());
            std::thread::spawn(move || writer(db, w, stop, bad))
        })
        .collect();
    let v = verify_beside.then(|| {
        let (db, stop, bad) = (db.clone(), stop.clone(), bad.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match db.verify(None) {
                    Ok(rows) => {
                        let f = api::printed(rows);
                        if !f.is_empty() {
                            bad.lock().unwrap().push(format!("verify: {f:?}"));
                        }
                    }
                    Err(e) => bad.lock().unwrap().push(format!("verify: {e:?}")),
                }
            }
        })
    });
    std::thread::sleep(Duration::from_millis(20));
    for i in 0..reindexes {
        let how = if i % 4 == 3 { " offline" } else { "" };
        let which = match i % 3 {
            0 => format!("reindex things index (owner) in (A | B){how}"),
            1 => format!("reindex things index (a) in A{how}"),
            _ => format!("reindex things{how}"),
        };
        run(&db, &which).unwrap();
        idle(&db);
        let found = run(&db, "from f in sys::verify_findings select f").unwrap();
        if !found.is_empty() {
            bad.lock()
                .unwrap()
                .push(format!("findings after {which}: {found:?}"));
            break;
        }
        if !bad.lock().unwrap().is_empty() {
            break;
        }
    }
    stop.store(true, Ordering::Release);
    let models: Vec<_> = ws.into_iter().map(|h| h.join().unwrap()).collect();
    if let Some(v) = v {
        v.join().unwrap();
    }
    let bad = bad.lock().unwrap().clone();
    assert!(bad.is_empty(), "{:#?}", &bad[..bad.len().min(8)]);
    assert!(db.write_path().pager().quarantined().is_empty());
    assert_eq!(Vec::<String>::new(), api::printed(db.verify(None).unwrap()));
    let every = |db: &Database| {
        for m in &models {
            for (k, r) in m {
                let by_key = run(
                    db,
                    &format!("from t in things where key t == things.key({k}) select t.owner"),
                )
                .unwrap();
                assert_eq!(vec![r.owner.to_string()], by_key, "row {k} by key");
                let by_owner = run(
                    db,
                    &format!("from t in things where t.owner == {} select t.id", r.owner),
                )
                .unwrap();
                assert!(
                    by_owner.iter().any(|s| s.contains(&k.to_string())),
                    "row {k} by owner"
                );
                if r.a {
                    let by_a = run(
                        db,
                        &format!(
                            r#"from t in things.A where t.a == "{}" select t.owner"#,
                            r.val
                        ),
                    )
                    .unwrap();
                    assert_eq!(vec![r.owner.to_string()], by_a, "row {k} by a");
                }
            }
        }
    };
    every(&db);
    db.close().unwrap();
    let db = open(&p);
    every(&db);
    assert_eq!(
        Vec::<String>::new(),
        api::printed(db.verify_holding().unwrap())
    );
    db.close().unwrap();
}

fn write_skew(name: &str, rounds: usize, reindex: bool) -> usize {
    let p = path(name);
    let db = open(&p);
    run(&db, "insert shifts { id: shifts.key(1), duty: 1 }").unwrap();
    run(&db, "insert shifts { id: shifts.key(2), duty: 1 }").unwrap();
    for i in 3..40 {
        run(
            &db,
            &format!("insert shifts {{ id: shifts.key({i}), duty: {} }}", 10 + i),
        )
        .unwrap();
    }
    let stop = Arc::new(AtomicBool::new(false));
    let job = {
        let (db, stop) = (db.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut n = 0;
            while reindex && !stop.load(Ordering::Acquire) {
                run(&db, "reindex shifts index (duty)").unwrap();
                idle(&db);
                n += 1;
            }
            n
        })
    };
    let start = Arc::new(Barrier::new(3));
    let mid = Arc::new(Barrier::new(2));
    let end = Arc::new(Barrier::new(3));
    let spawn = |me: i64| {
        let (db, start, mid, end) = (db.clone(), start.clone(), mid.clone(), end.clone());
        std::thread::spawn(move || {
            for _ in 0..rounds {
                start.wait();
                let mut t = db.begin(&TxnOptions::default()).unwrap();
                let o = TxnCallOptions::default();
                let read = t.query_text("from s in shifts where s.duty == 1 select s.id", &[], &o);
                mid.wait();
                let n = read.map(|mut r| r.remove(0).count()).unwrap_or(0);
                if n >= 2 {
                    let _w = t.query_text(
                        &format!("update s in shifts where key s == shifts.key({me}) set Shift {{ s with duty: 0 }}"),
                        &[], &o,
                    );
                }
                let _c = t.commit(None);
                end.wait();
            }
        })
    };
    let (a, b) = (spawn(1), spawn(2));
    let mut broken = 0;
    for r in 0..rounds {
        start.wait();
        end.wait();
        let on = run(&db, "from s in shifts where s.duty == 1 select s.id")
            .unwrap()
            .len();
        let mut on_heap = 0;
        for k in [1, 2] {
            let d = run(
                &db,
                &format!("from s in shifts where key s == shifts.key({k}) select s.duty"),
            )
            .unwrap();
            on_heap += d.iter().filter(|d| *d == "1").count();
        }
        assert_eq!(
            on, on_heap,
            "round {r}: index gives {on}, key lookups {on_heap}"
        );
        if on == 0 {
            broken += 1;
        }
        for k in [1, 2] {
            run(&db, &format!("update s in shifts where key s == shifts.key({k}) set Shift {{ s with duty: 1 }}")).unwrap();
        }
    }
    a.join().unwrap();
    b.join().unwrap();
    stop.store(true, Ordering::Release);
    let jobs = job.join().unwrap();
    db.close().unwrap();
    assert!(!reindex || jobs > 0, "{name}: no rebuild ran");
    broken
}

fn phantom(name: &str, rounds: i64, reindex: bool) -> usize {
    let p = path(name);
    let db = open(&p);
    for i in 1..40 {
        run(
            &db,
            &format!("insert shifts {{ id: shifts.key({i}), duty: -{i} }}"),
        )
        .unwrap();
    }
    let stop = Arc::new(AtomicBool::new(false));
    let job = {
        let (db, stop) = (db.clone(), stop.clone());
        std::thread::spawn(move || {
            while reindex && !stop.load(Ordering::Acquire) {
                run(&db, "reindex shifts index (duty)").unwrap();
                idle(&db);
            }
        })
    };
    let (start, mid) = (Arc::new(Barrier::new(3)), Arc::new(Barrier::new(2)));
    let spawn = |me: i64| {
        let (db, start, mid) = (db.clone(), start.clone(), mid.clone());
        std::thread::spawn(move || {
            for r in 1..=rounds {
                start.wait();
                let mut t = db.begin(&TxnOptions::default()).unwrap();
                let o = TxnCallOptions::default();
                let n = t
                    .query_text(
                        &format!("from s in shifts where s.duty == {r} select s.id"),
                        &[],
                        &o,
                    )
                    .map(|mut x| x.remove(0).count());
                mid.wait();
                if let Ok(0) = n {
                    let _w = t.query_text(
                        &format!(
                            "insert shifts {{ id: shifts.key({}), duty: {r} }}",
                            1000 * r + me
                        ),
                        &[],
                        &o,
                    );
                }
                let _c = t.commit(None);
                start.wait();
            }
        })
    };
    let (a, b) = (spawn(1), spawn(2));
    let mut broken = 0;
    for r in 1..=rounds {
        start.wait();
        start.wait();
        let n = run(
            &db,
            &format!("from s in shifts where s.duty == {r} select s.id"),
        )
        .unwrap()
        .len();
        assert!(n >= 1, "round {r}: no row, an entry is lost");
        if n > 1 {
            broken += 1;
        }
    }
    a.join().unwrap();
    b.join().unwrap();
    stop.store(true, Ordering::Release);
    job.join().unwrap();
    db.close().unwrap();
    broken
}

#[test]
fn stress_writers() {
    stress("stress", 3, false);
}

#[test]
fn stress_writers_and_verify() {
    stress("stress_verify", 2, true);
}

#[test]
fn write_skew_through_a_reindexing_index() {
    assert_eq!(
        0,
        write_skew("skew", 12, true),
        "rounds where both sides left duty"
    );
}

#[test]
fn phantom_through_a_reindexing_index() {
    assert_eq!(
        0,
        phantom("phantom", 12, true),
        "rounds where both sides inserted"
    );
}

fn past_gate_rounds(db: &Database, rounds: u32) {
    let bulk = "update t in things.A set A { t with owner: t.owner + 100000 }";
    for r in 0..rounds {
        let before = db.write_path().pager().appended();
        let d = db.clone();
        let writer = std::thread::spawn(move || run(&d, bulk));
        while db.write_path().pager().appended() == before {
            std::thread::yield_now();
        }
        run(db, "reindex things index (owner) in (A | B) offline").unwrap();
        writer.join().unwrap().unwrap();
        idle(db);
        let findings = api::printed(db.verify(None).unwrap());
        assert_eq!(Vec::<String>::new(), findings, "round {r}");
    }
}

#[test]
fn a_statement_past_the_gate_loses_no_entry() {
    let db = open(&path("past_gate"));
    let note = "n".repeat(1000);
    let mut x = db.begin(&TxnOptions::default()).unwrap();
    for k in 1..=200 {
        let t = format!(
            r#"insert things.A {{ id: things.key({k}), owner: {k}, note: "{note}", a: "{k}" }}"#
        );
        x.query_text(&t, &[], &TxnCallOptions::default()).unwrap();
    }
    x.commit(None).unwrap();
    db.on_reindex_phase(Arc::new(|phase| {
        if phase == "build" {
            std::thread::sleep(Duration::from_millis(30));
        }
    }));
    past_gate_rounds(&db, 3);
    assert!(db.write_path().pager().quarantined().is_empty());
    db.close().unwrap();
}

/// Hold the rebuild in its build phase until the first flag is set, at most 10 s, then set the
/// second if it was not.
fn hold_build(db: &Database) -> (Arc<AtomicBool>, Arc<AtomicBool>) {
    let (go, timed_out) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let (g, t) = (go.clone(), timed_out.clone());
    db.on_reindex_phase(Arc::new(move |phase| {
        if phase == "build" {
            let start = Instant::now();
            while !g.load(Ordering::Acquire) {
                if start.elapsed() > Duration::from_secs(10) {
                    t.store(true, Ordering::Release);
                    return;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }));
    (go, timed_out)
}

const GATED_INSERT: &str = r#"insert things.A { id: things.key(2), owner: 2, note: "n", a: "y" }"#;

fn offline_with_one_row(name: &str) -> (Database, Arc<AtomicBool>, Arc<AtomicBool>) {
    let db = open(&path(name));
    run(
        &db,
        r#"insert things.A { id: things.key(1), owner: 1, note: "n", a: "x" }"#,
    )
    .unwrap();
    let (go, timed_out) = hold_build(&db);
    run(&db, "reindex things index (owner) in (A | B) offline").unwrap();
    while run(
        &db,
        r#"from j in sys::jobs where j.phase == "build" select j.id"#,
    )
    .unwrap()
    .is_empty()
    {
        std::thread::sleep(Duration::from_millis(1));
    }
    (db, go, timed_out)
}

#[test]
fn a_gated_write_stops_at_its_deadline() {
    let (db, go, _) = offline_with_one_row("gate_deadline");
    let start = Instant::now();
    let r = db.query_text(
        GATED_INSERT,
        &[],
        &CallOptions {
            deadline: Some(SystemTime::now() + Duration::from_millis(200)),
            ..CallOptions::default()
        },
    );
    let took = start.elapsed();
    go.store(true, Ordering::Release);
    assert!(r.is_err(), "the write ran through the gate");
    assert!(
        took >= Duration::from_millis(200) && took < Duration::from_secs(1),
        "{took:?}"
    );
    idle(&db);
    db.close().unwrap();
}

#[test]
fn a_gated_write_stops_when_cancelled() {
    let (db, go, _) = offline_with_one_row("gate_cancel");
    let token = CancelToken::new();
    let (w, t) = (db.clone(), token.clone());
    let writer = std::thread::spawn(move || {
        let opts = CallOptions {
            cancel: Some(t),
            ..CallOptions::default()
        };
        w.query_text(GATED_INSERT, &[], &opts).is_err()
    });
    std::thread::sleep(Duration::from_millis(30));
    let start = Instant::now();
    token.cancel();
    let refused = writer.join().unwrap();
    let took = start.elapsed();
    go.store(true, Ordering::Release);
    assert!(refused, "the write ran through the gate");
    assert!(took < Duration::from_secs(1), "{took:?}");
    idle(&db);
    db.close().unwrap();
}

#[test]
fn close_with_a_gated_writer_does_not_wait_out_the_rebuild() {
    let (db, go, timed_out) = offline_with_one_row("gate_close");
    let w = db.clone();
    let release = go;
    let writer = std::thread::spawn(move || {
        let r = w.query_text(GATED_INSERT, &[], &CallOptions::default());
        release.store(true, Ordering::Release);
        r.is_err()
    });
    std::thread::sleep(Duration::from_millis(30));
    db.close().unwrap();
    assert!(
        writer.join().unwrap(),
        "a writer the gate holds is refused at close"
    );
    assert!(
        !timed_out.load(Ordering::Acquire),
        "close waited for the rebuild"
    );
}

#[test]
fn a_closed_job_leaves_a_clean_file() {
    let p = path("gate_close_reopen");
    let db = open(&p);
    run(
        &db,
        r#"insert things.A { id: things.key(1), owner: 1, note: "n", a: "x" }"#,
    )
    .unwrap();
    let (go, _) = hold_build(&db);
    run(&db, "reindex things index (owner) in (A | B) offline").unwrap();
    go.store(true, Ordering::Release);
    db.close().unwrap();
    let db = open(&p);
    assert_eq!(
        Vec::<String>::new(),
        api::printed(db.verify_holding().unwrap())
    );
    db.close().unwrap();
}

#[test]
#[ignore = "the job keeps the engine open until its rebuild ends"]
fn drop_without_close_then_reopen() {
    let p = path("drop");
    let db = open(&p);
    run(
        &db,
        r#"insert things.A { id: things.key(1), owner: 1, note: "n", a: "x" }"#,
    )
    .unwrap();
    let (go, _) = hold_build(&db);
    run(&db, "reindex things").unwrap();
    std::thread::sleep(Duration::from_millis(20));
    drop(db);
    let again = Database::open(&p, &Log::new([APP]), &api::options(4));
    go.store(true, Ordering::Release);
    assert!(
        again.is_ok(),
        "the file is still held after every handle dropped"
    );
    again.unwrap().close().unwrap();
}

const SEQ: &str = r"module app
add module app
add type P = { k: I64, v: I64 }
add relation ps : P key k
  index (v)
add type G = { id: Key gs, v: I64 }
add relation gs : G key id generate sequence
";

fn seq_open(rt: SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt, "s.adt", &Log::new([SEQ]), &api::options(2)).unwrap()
}

fn wait_jobs(rt: &SimRuntime, db: &Database<SimRuntime>) {
    while !api::printed(api::result(db, "from j in sys::jobs select j.id")).is_empty() {
        for _ in 0..64 {
            rt.yield_now();
        }
    }
}

type Times = Arc<Mutex<Vec<(String, adtb::runtime::Instant)>>>;

fn seq_body(rt: &SimRuntime, times: Option<Times>) {
    let db = seq_open(rt.clone());
    for i in 1..=40 {
        api::results(&db, &format!("insert ps {{ k: {i}, v: {i} }}"));
    }
    let (w, clock) = (db.clone(), rt.clone());
    db.on_reindex_phase(Arc::new(move |phase| {
        if phase == "build" {
            api::results(&w, "insert gs { v: 1 }");
        }
        w.write_path().pager().flush_log();
        if let Some(t) = &times {
            t.lock().unwrap().push((phase.to_owned(), clock.now()));
        }
    }));
    api::results(&db, "reindex ps index (v)");
    wait_jobs(rt, &db);
    db.close().unwrap();
}

#[test]
fn a_crash_at_the_swap_after_a_sequence_is_used_leaves_a_clean_file() {
    let times = Arc::new(Mutex::new(Vec::new()));
    let t2 = times.clone();
    let mut sim = Sim::new(5, Config::default());
    api::run(&mut sim, move |rt| seq_body(&rt, Some(t2)));
    let at = times
        .lock()
        .unwrap()
        .iter()
        .find(|(p, _)| p == "swap")
        .unwrap()
        .1;
    crash::silence_fatal_panics();
    let mut sim = Sim::new(5, Config::default());
    sim.crash_at(at);
    assert_eq!(Outcome::Crashed, sim.run(|rt| seq_body(&rt, None)));
    api::run(&mut sim, |rt| {
        let db = seq_open(rt);
        assert_eq!(
            Vec::<String>::new(),
            api::printed(db.verify_holding().unwrap())
        );
        db.close().unwrap();
    });
}

fn crash_job_body(rt: &SimRuntime, times: Option<Arc<Mutex<Vec<adtb::runtime::Instant>>>>) {
    let db = seq_open(rt.clone());
    for i in 1..=100 {
        api::results(&db, &format!("insert ps {{ k: {i}, v: {i} }}"));
    }
    if let Some(t) = &times {
        t.lock().unwrap().push(rt.now());
    }
    api::results(&db, "reindex ps");
    wait_jobs(rt, &db);
    if let Some(t) = &times {
        t.lock().unwrap().push(rt.now());
    }
    db.close().unwrap();
}

#[test]
fn a_crash_anywhere_in_the_job_leaves_a_clean_file() {
    let times = Arc::new(Mutex::new(Vec::new()));
    let t2 = times.clone();
    let mut sim = Sim::new(7, Config::default());
    api::run(&mut sim, move |rt| crash_job_body(&rt, Some(t2)));
    let (from, to) = {
        let t = times.lock().unwrap();
        (t[0], t[1])
    };
    let span = to.saturating_since(from);
    crash::silence_fatal_panics();
    let points = 6u32;
    for i in 1..points {
        let mut sim = Sim::new(7, Config::default());
        sim.crash_at(from + span * i / points);
        assert_eq!(
            Outcome::Crashed,
            sim.run(|rt| crash_job_body(&rt, None)),
            "point {i}"
        );
        api::run(&mut sim, move |rt| {
            let db = seq_open(rt);
            assert_eq!(
                Vec::<String>::new(),
                api::printed(db.verify_holding().unwrap()),
                "point {i}"
            );
            assert_eq!(
                1,
                api::printed(api::result(&db, "from p in ps where p.v == 77 select p.k")).len(),
                "point {i}"
            );
            db.close().unwrap();
        });
    }
}

const HIST: &str = r"module app
add module app
add type T = { id: Key ts, v: I64 }
add relation ts : T key id generate sequence
  index (v)
";

const HIST_FILE: &str = "h.adt";

fn hist_open(rt: SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt, HIST_FILE, &Log::new([HIST]), &api::options(2)).unwrap()
}

/// Seed a relation with history. Returns the text of a read of the past through the history key
/// index, which answers `1007`.
fn seed_history(db: &Database<SimRuntime>) -> String {
    api::results(db, "set setting RetentionWindow = { length: 2h }");
    for i in 1..=20 {
        api::results(db, &format!("insert ts {{ id: ts.key({i}), v: {i} }}"));
    }
    let commit = |t: &str| {
        let mut x = db.begin(&TxnOptions::default()).unwrap();
        api::in_txn_results(&mut x, t);
        x.commit(None).unwrap()
    };
    let first = commit("update t in ts where key t == ts.key(7) set T { t with v: 1007 }");
    commit("update t in ts where key t == ts.key(7) set T { t with v: 2007 }");
    api::results(db, "vacuum ts");
    format!(
        "at commit Position {{ epoch: {}, lsn: {} }} {{ from n in [ts.key(7)] select n.v }}",
        first.epoch, first.lsn
    )
}

fn history_root(db: &Database<SimRuntime>) -> adtb::storage::Pid {
    db.write_path()
        .history_index_named("app::ts")
        .unwrap()
        .tree()
        .root()
}

fn marks(db: &Database<SimRuntime>) -> BTreeSet<Quarantine> {
    db.write_path().pager().quarantined()
}

fn insert_ok(db: &Database<SimRuntime>, k: u32) -> bool {
    let t = format!("insert ts {{ id: ts.key({k}), v: {k} }}");
    db.query_text(&t, &[], &CallOptions::default()).is_ok()
}

#[test]
fn a_damaged_history_key_index_is_rebuilt_from_the_history_heap() {
    let mut sim = Sim::new(3, Config::default());
    let past = Arc::new(Mutex::new(String::new()));
    let keep = past.clone();
    api::run(&mut sim, move |rt| {
        let db = hist_open(rt.clone());
        let read = seed_history(&db);
        assert_eq!(vec!["1007"], api::query(&db, &read), "before the damage");
        db.damage_page(history_root(&db), Damage::Checksum);
        let refused = db.query_text(&read, &[], &CallOptions::default());
        assert!(
            refused.is_err(),
            "a read of the past through a damaged tree"
        );
        assert_eq!(1, marks(&db).len(), "{:?}", marks(&db));
        assert!(
            !insert_ok(&db, 500),
            "a write while the key index is marked"
        );
        api::results(&db, "reindex ts");
        wait_jobs(&rt, &db);
        assert!(db.write_path().pager().quarantined().is_empty());
        assert_eq!(
            Vec::<String>::new(),
            api::printed(api::result(&db, "from f in sys::verify_findings select f"))
        );
        assert!(insert_ok(&db, 501), "a write after the rebuild");
        assert_eq!(vec!["1007"], api::query(&db, &read), "after the rebuild");
        assert_eq!(
            Vec::<String>::new(),
            api::printed(db.verify_holding().unwrap())
        );
        *keep.lock().unwrap() = read;
        db.close().unwrap();
    });
    api::run(&mut sim, move |rt| {
        let db = hist_open(rt);
        assert!(db.write_path().pager().quarantined().is_empty());
        assert_eq!(
            vec!["1007"],
            api::query(&db, &past.lock().unwrap()),
            "after a restart"
        );
        assert_eq!(
            Vec::<String>::new(),
            api::printed(db.verify_holding().unwrap())
        );
        db.close().unwrap();
    });
}

#[test]
fn a_history_page_damaged_during_the_build_is_not_released_unrepaired() {
    api::once(3, |rt| {
        let db = hist_open(rt.clone());
        let read = seed_history(&db);
        let (d, r) = (db.clone(), read.clone());
        let once = AtomicBool::new(false);
        db.on_reindex_phase(Arc::new(move |phase| {
            if phase == "build" && !once.swap(true, Ordering::AcqRel) {
                d.damage_page(history_root(&d), Damage::Checksum);
                assert!(d.query_text(&r, &[], &CallOptions::default()).is_err());
            }
        }));
        api::results(&db, "reindex ts");
        wait_jobs(&rt, &db);
        assert!(db.write_path().pager().quarantined().is_empty());
        assert_eq!(vec!["1007"], api::query(&db, &read));
        assert_eq!(
            Vec::<String>::new(),
            api::printed(db.verify_holding().unwrap())
        );
        db.close().unwrap();
    });
}

#[test]
fn a_history_tree_that_cannot_be_rebuilt_keeps_the_mark() {
    api::once(3, |rt| {
        let db = hist_open(rt.clone());
        let read = seed_history(&db);
        let w = db.write_path();
        let heap = w.history_heap_named("app::ts", "T").unwrap();
        let page = heap.pages(w.pager()).unwrap()[0];
        db.damage_page(history_root(&db), Damage::Checksum);
        assert!(db.query_text(&read, &[], &CallOptions::default()).is_err());
        let before = marks(&db);
        assert_eq!(1, before.len(), "{before:?}");
        db.damage_page(page, Damage::Checksum);
        api::results(&db, "reindex ts");
        wait_jobs(&rt, &db);
        assert!(
            marks(&db).is_superset(&before),
            "the mark stays: {:?}",
            marks(&db)
        );
        let found = api::printed(api::result(&db, "from f in sys::verify_findings select f"));
        assert!(found.iter().any(|f| f.contains("Structure")), "{found:?}");
        db.close().unwrap();
    });
}

fn heap_mark(db: &Database<SimRuntime>) -> Quarantine {
    let w = db.write_path();
    Quarantine::Heap(w.heap_named("app::ts", "T").unwrap().id().0)
}

#[test]
fn a_heap_marked_while_the_rebuild_waits_stops_it() {
    api::once(3, |rt| {
        let db = hist_open(rt.clone());
        for i in 1..=5 {
            api::results(&db, &format!("insert ts {{ id: ts.key({i}), v: {i} }}"));
        }
        let phases = Arc::new(Mutex::new(Vec::new()));
        let saw = phases.clone();
        db.on_reindex_phase(Arc::new(move |p| saw.lock().unwrap().push(p.to_owned())));
        let held = db.write_path().holding_off_vacuum();
        api::results(&db, "reindex ts index (v)");
        for _ in 0..2000 {
            rt.yield_now();
        }
        assert!(phases.lock().unwrap().is_empty());
        db.write_path().pager().quarantine(heap_mark(&db)).unwrap();
        drop(held);
        wait_jobs(&rt, &db);
        assert!(
            phases.lock().unwrap().iter().all(|p| p != "swap"),
            "{phases:?}"
        );
        let found = api::printed(api::result(&db, "from f in sys::verify_findings select f"));
        assert_eq!(1, found.len(), "{found:?}");
        db.close().unwrap();
    });
}

#[test]
fn a_heap_marked_during_the_build_stops_the_swap() {
    api::once(3, |rt| {
        let db = hist_open(rt.clone());
        for i in 1..=5 {
            api::results(&db, &format!("insert ts {{ id: ts.key({i}), v: {i} }}"));
        }
        let d = db.clone();
        let mark = heap_mark(&db);
        let reclaimed = Arc::new(AtomicBool::new(false));
        let r = reclaimed.clone();
        db.on_reindex_phase(Arc::new(move |p| {
            if p == "build" {
                d.write_path().pager().quarantine(mark).unwrap();
            }
            if p == "reclaim" {
                r.store(true, Ordering::Release);
            }
        }));
        api::results(&db, "reindex ts index (v)");
        wait_jobs(&rt, &db);
        assert!(!reclaimed.load(Ordering::Acquire), "the swap ran");
        db.close().unwrap();
    });
}

#[test]
fn a_panic_after_the_swap_frees_no_page_of_the_new_index() {
    api::once(3, |rt| {
        let db = hist_open(rt.clone());
        for i in 1..=400 {
            api::results(&db, &format!("insert ts {{ id: ts.key({i}), v: {i} }}"));
        }
        let index = db.write_path().index_named("app::ts", "index (v)").unwrap();
        let tree = index.tree();
        let pages = tree.pages(db.write_path().pager()).unwrap();
        assert!(pages.len() > 1, "a tree of one page has no page to lose");
        db.damage_page(*pages.last().unwrap(), Damage::Checksum);
        db.on_reindex_phase(Arc::new(|p| assert_ne!("sweep", p, "the injected failure")));
        api::results(&db, "reindex ts index (v)");
        wait_jobs(&rt, &db);
        for i in [1, 77, 400] {
            let got = api::query(&db, &format!("from t in ts where t.v == {i} select t.v"));
            assert_eq!(vec![i.to_string()], got);
        }
        assert!(insert_ok(&db, 900), "the index takes a write");
        assert!(db.write_path().pager().quarantined().is_empty());
        let reached = tree.pages(db.write_path().pager()).unwrap();
        for p in reached {
            assert!(
                !db.write_path().pager().is_free(p).unwrap(),
                "{p:?} was freed"
            );
        }
        db.close().unwrap();
    });
}

#[test]
fn a_job_joins_the_threads_of_jobs_before_it() {
    api::once(3, |rt| {
        let db = hist_open(rt.clone());
        api::results(&db, "insert ts { id: ts.key(1), v: 1 }");
        for _ in 0..4 {
            api::results(&db, "reindex ts index (v)");
            wait_jobs(&rt, &db);
            for _ in 0..2000 {
                rt.yield_now();
            }
        }
        assert_eq!(1, db.reindex_threads());
        db.close().unwrap();
    });
}

#[test]
fn a_read_runs_while_gated_writes_fill_the_workers() {
    api::once(3, |rt| {
        let db = hist_open(rt.clone());
        for i in 1..=20 {
            api::results(&db, &format!("insert ts {{ id: ts.key({i}), v: {i} }}"));
        }
        let session = Arc::new(db.role_authority().session("owner").unwrap());
        let read_done = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(None));
        let (clock, s2, rd, out) = (rt.clone(), session, read_done, seen.clone());
        let threads = Arc::new(Mutex::new(Vec::new()));
        let th = threads.clone();
        db.on_reindex_phase(Arc::new(move |phase| {
            if phase == "build" {
                for i in 0..2 {
                    let (c, s) = (clock.clone(), s2.clone());
                    th.lock().unwrap().push(clock.spawn("w", move || {
                        api::task(
                            &c,
                            &s,
                            &format!("insert ts {{ id: ts.key({}), v: 1 }}", 100 + i),
                        )
                        .unwrap();
                    }));
                }
                for _ in 0..2000 {
                    clock.yield_now();
                }
                let (c, s, r) = (clock.clone(), s2.clone(), rd.clone());
                th.lock().unwrap().push(clock.spawn("r", move || {
                    api::task(&c, &s, "from t in ts select t.v").unwrap();
                    r.store(true, Ordering::Release);
                }));
                for _ in 0..200_000 {
                    if rd.load(Ordering::Acquire) {
                        break;
                    }
                    clock.yield_now();
                }
                *out.lock().unwrap() = Some(rd.load(Ordering::Acquire));
            }
        }));
        api::results(&db, "reindex ts index (v) offline");
        wait_jobs(&rt, &db);
        for t in std::mem::take(&mut *threads.lock().unwrap()) {
            rt.join(t);
        }
        let ran = seen.lock().unwrap().unwrap();
        assert_eq!(
            2,
            api::query(&db, "from t in ts where t.v == 1 select t.id").len() - 1,
            "both writes ran after the gate"
        );
        db.close().unwrap();
        assert!(
            ran,
            "a read task did not run while writes waited on an offline rebuild"
        );
    });
}
