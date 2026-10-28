use std::sync::Barrier;

use adtb_testkit::profile::{count, sweep};

use adtb::{CallOptions, Called, CreateOptions, Log, OpenOptions, Registration, Value};

const LOG: &str = "module counters\nadd module counters\nadd type Counter = { id: Key counters, value: I64 }\nadd relation counters : Counter key id generate sequence\n";
const APP: &str = "import counters\nquery value () -> Set I64 = from c in counters select c.value\ntxn increment () -> () = transact update c in counters set { c with value: c.value + 1 }\n";

#[test]
fn two_committed_updates_advance_twice() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("update-concurrency");
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("counter.adt");
    let log = Log::new([LOG]);
    let opts = OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(2),
        registrations: vec![Registration::new("counter", APP)],
        ..OpenOptions::default()
    };
    let db = adtb::open(&path, &log, &opts).unwrap();
    db.query_text("insert counters { value: 0 }", &[], &CallOptions::default())
        .unwrap();
    let calls = db.registered("counter").unwrap();
    let pairs = i64::try_from(count(64)).unwrap();
    for iteration in 0..pairs {
        let barrier = Barrier::new(2);
        let (a, b) = std::thread::scope(|scope| {
            let a = scope.spawn(|| {
                barrier.wait();
                calls.call(2, &[], &CallOptions::default()).unwrap()
            });
            let b = scope.spawn(|| {
                barrier.wait();
                calls.call(2, &[], &CallOptions::default()).unwrap()
            });
            (a.join().unwrap(), b.join().unwrap())
        });
        let Called::Rows(rows) = calls.call(1, &[], &CallOptions::default()).unwrap() else {
            panic!("query returns rows")
        };
        let rows: Vec<_> = rows.collect::<Result<_, _>>().unwrap();
        assert_eq!(
            rows,
            vec![Value::Int((iteration + 1) * 2)],
            "iteration={iteration}, a={a:?}, b={b:?}"
        );
    }
    db.close().unwrap();
    let db = adtb::open(&path, &log, &opts).unwrap();
    let calls = db.registered("counter").unwrap();
    let Called::Rows(rows) = calls.call(1, &[], &CallOptions::default()).unwrap() else {
        panic!("query returns rows");
    };
    assert_eq!(
        rows.collect::<Result<Vec<_>, _>>().unwrap(),
        vec![Value::Int(pairs * 2)]
    );
    db.close().unwrap();
}

#[test]
fn simulated_committed_updates_advance_once_each() {
    use adtb::runtime::Threads;
    use adtb_sim::{Config, Outcome, Sim};
    for seed in sweep(4..5) {
        let mut sim = Sim::new(
            seed,
            Config {
                switch_rate: 0.3,
                ..Config::default()
            },
        );
        let outcome = sim.run(|rt| {
            let opts = OpenOptions {
                create: Some(CreateOptions::default()),
                workers: Some(2),
                registrations: vec![Registration::new("counter", APP)],
                ..OpenOptions::default()
            };
            let db = adtb::Database::open_in(rt.clone(), "counter.adt", &Log::new([LOG]), &opts)
                .unwrap();
            db.query_text("insert counters { value: 0 }", &[], &CallOptions::default())
                .unwrap();
            let calls = db.registered("counter").unwrap();
            let threads: Vec<_> = (0..2)
                .map(|thread| {
                    let calls = calls.clone();
                    rt.spawn("increment", move || {
                        for iteration in 0..8 {
                            calls
                                .call(2, &[], &CallOptions::default())
                                .unwrap_or_else(|e| {
                                    panic!(
                                        "seed={seed} thread={thread} iteration={iteration}: {e:?}"
                                    )
                                });
                        }
                    })
                })
                .collect();
            for thread in threads {
                rt.join(thread);
            }
            let Called::Rows(rows) = calls.call(1, &[], &CallOptions::default()).unwrap() else {
                panic!("query returns rows")
            };
            assert_eq!(
                rows.collect::<Result<Vec<_>, _>>().unwrap(),
                vec![Value::Int(16)],
                "seed={seed}"
            );
            db.close().unwrap();
        });
        assert_eq!(outcome, Outcome::Finished, "seed={seed}");
    }
}
