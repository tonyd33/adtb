//! The in-process API over the simulator: open, ad hoc text, interactive transactions and the
//! retrying closure.

use crate::test_world::load as open;
use adtb_harness::process::run;

use crate::{CallOptions, Database, Log, Value};
use adtb_lang::types::{Base, Ty};
use adtb_sim::{Config, Sim, SimRuntime};

use crate::test_world::load::options;

const SHOP: &str = r"module shop
add module shop
add type Note = { id: Key notes, body: Text }
add relation notes : Note key id generate sequence
add type Customer = { id: Key customers, name: Text }
add relation customers : Customer key id generate sequence
add type Payment { id: Key payments, customer: Key customers }
  = Card { last4: Text }
  | Invoice { due: Text }
add relation payments : Payment
  key id generate sequence
  index (customer)
add constraint paid =
  from c in customers
  where not exists p in payments where p.customer == key c
  select c
";

/// Insert a customer named `n` with a card, as one statement.
const SIGN_UP: &str = r#"transact
  let c = insert customers { name: n }
  insert payments.Card { customer: key c, last4: "4242" }
  return key c"#;

fn log() -> Log {
    Log::new([SHOP])
}

fn text(t: &str) -> Value {
    Value::text(t)
}

fn name(n: &str) -> [(&'static str, Ty, Value); 1] {
    [("n", Ty::Base(Base::Text), text(n))]
}

fn sign_up(db: &Database<SimRuntime>, n: &str) {
    db.query_text(SIGN_UP, &name(n), &CallOptions::default())
        .unwrap_or_else(|e| panic!("signing up {n} fails: {e:?}"));
}

/// Returns what `verify` finds in the closed file `app.adt`, and whether its counters page is
/// named.
fn verified(rt: SimRuntime) -> (crate::verify::Report, bool) {
    let options = crate::storage::Options {
        page_size: 8192,
        threads: 4,
        ..crate::storage::Options::new("app.adt")
    };
    let (pager, _) = crate::storage::Pager::open(rt.clone(), &options).expect("the pager");
    let (catalog, txns) = crate::catalog::Catalog::open(rt, &pager).expect("the catalog");
    let report = crate::verify::verify(&pager, &catalog.inventory(&txns.commit_log()));
    (report, catalog.sequences().is_some())
}

#[test]
fn a_closed_file_verifies_clean_with_every_page_accounted_for() {
    let mut sim = Sim::new(7, Config::default());
    run(&mut sim, |rt| {
        let db = open::opened(&rt, open::FILE, &log(), &options(4));
        sign_up(&db, "ann");
        db.close().expect("close");
    });
    run(&mut sim, |rt| {
        drop(open::opened(&rt, open::FILE, &log(), &options(4)));
    });
    let (report, named) = run(&mut sim, verified);
    assert!(named, "the counters page is named");
    assert!(report.is_clean() && report.complete, "{report:#?}");
}

#[test]
fn a_crash_as_the_first_open_names_its_counters_page_leaks_no_page() {
    let seed = 11;
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    run(&mut sim, |rt| {
        std::mem::forget(open::opened(&rt, open::FILE, &log(), &options(4)));
    });
    let recording = sim.recording();
    let flushes = recording.flushes();
    let from = flushes[flushes.len().saturating_sub(3)];
    for at in from..=recording.len() {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        let mut sim = Sim::with_files(seed + at as u64, Config::default(), files);
        run(&mut sim, |rt| {
            open::opened(&rt, open::FILE, &log(), &options(4))
                .close()
                .expect("close");
        });
        let (report, named) = run(&mut sim, verified);
        assert!(named, "crash at {at}: the counters page is named");
        assert!(
            report.is_clean() && report.complete,
            "crash at {at}: {report:#?}"
        );
    }
}
