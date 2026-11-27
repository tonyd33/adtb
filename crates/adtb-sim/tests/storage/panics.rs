use std::panic::{self, AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use adtb::runtime::Threads;
use adtb::{CallOptions, Log};
use adtb_sim::{Config, Outcome, Sim};
use adtb_simkit::open;
use adtb_sys::pages::PageMemory;

const PAGE: usize = 4096;
const APP: &str = r"module app
add module app
add type User = { id: Key users }
add relation users : User key id generate sequence
";

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "goes if the test takes its latch through a pager"
)]
fn a_panic_under_a_page_latch_crashes_the_run_before_another_thread_takes_the_latch() {
    let mut sim = Sim::new(7, Config::default());
    let mem = Arc::new(PageMemory::reserve(PAGE, 1));
    let taken = Arc::new(AtomicBool::new(false));
    let t = taken.clone();
    let run = catch_unwind(AssertUnwindSafe(|| {
        sim.run(move |rt| {
            let (holder_rt, holder_mem) = (rt.clone(), mem.clone());
            let holder = rt.spawn("holder", move || {
                let latch = holder_mem.lock_exclusive(0, &holder_rt);
                holder_rt.yield_now();
                // The catch stands for any frame that would stop the unwind.
                let caught = catch_unwind(|| panic!("an engine invariant"));
                drop(caught);
                drop(latch);
                holder_rt.yield_now();
            });
            let waiter_rt = rt.clone();
            let waiter = rt.spawn("waiter", move || {
                let _latch = mem.lock_exclusive(0, &waiter_rt);
                t.store(true, Ordering::SeqCst);
            });
            rt.join(holder);
            rt.join(waiter);
        })
    }));
    let payload = run.expect_err("the panic ends the run");
    assert_eq!(
        Some("an engine invariant".to_owned()),
        adtb::testing::panic_text(&*payload)
    );
    assert!(
        !taken.load(Ordering::SeqCst),
        "a thread took the latch after the panic"
    );
}

#[test]
fn the_engines_hook_runs_the_hook_it_replaced_and_ends_only_a_panic_on_an_engine_thread() {
    let reports = Arc::new(Mutex::new(Vec::new()));
    let r = reports.clone();
    panic::set_hook(Box::new(move |info| {
        let text = adtb::testing::panic_text(info.payload()).unwrap_or_default();
        r.lock().unwrap().push(text);
    }));
    let mut sim = Sim::new(3, Config::default());
    let outcome = sim.run(|rt| {
        let db = open::opened(&rt, open::FILE, &Log::new([APP]), &open::options(1));
        let caught = catch_unwind(|| panic!("the embedder's own"));
        assert_eq!(
            Some("the embedder's own".to_owned()),
            caught.err().and_then(|p| adtb::testing::panic_text(&*p))
        );
        db.query_text("from u in users select u.id", &[], &CallOptions::default())
            .expect("the engine answers after the embedder's panic");
        db.close().expect("the database closes");
    });
    assert_eq!(Outcome::Finished, outcome);
    let run = catch_unwind(AssertUnwindSafe(|| {
        sim.run(|rt| {
            let engine = rt.spawn("engine", || {
                drop(catch_unwind(|| panic!("an engine thread's")));
            });
            rt.join(engine);
        })
    }));
    drop(panic::take_hook());
    let payload = run.expect_err("the engine thread's panic ends the run");
    assert_eq!(
        Some("an engine thread's".to_owned()),
        adtb::testing::panic_text(&*payload)
    );
    assert_eq!(
        vec![
            "the embedder's own".to_owned(),
            "an engine thread's".to_owned()
        ],
        *reports.lock().unwrap()
    );
}
