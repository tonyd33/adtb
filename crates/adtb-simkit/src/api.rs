use adtb::{
    AdHocTransaction, CallOptions, CreateOptions, Database, DynRows, Error, OpenOptions,
    RejectReason, TxnCallOptions,
};
use adtb_lang::textual::print;
use adtb_sim::{Config, Outcome, Sim, SimRuntime};

/// Returns the options that open the database with `workers` workers, creating it if it does not
/// exist.
pub fn options(workers: usize) -> OpenOptions {
    OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(workers),
        ..OpenOptions::default()
    }
}

/// Runs `body` as one process of `sim` and returns what it returns.
pub fn run<T>(sim: &mut Sim, body: impl FnOnce(SimRuntime) -> T) -> T {
    finished(sim, body, "the run")
}

/// Runs `body` as the one process of a new simulation seeded `seed` and returns what it returns.
pub fn once<T>(seed: u64, body: impl FnOnce(SimRuntime) -> T) -> T {
    let mut sim = Sim::new(seed, Config::default());
    finished(&mut sim, body, &format!("the run of seed {seed}"))
}

fn finished<T>(sim: &mut Sim, body: impl FnOnce(SimRuntime) -> T, what: &str) -> T {
    let mut got = None;
    let outcome = sim.run(|rt| got = Some(body(rt)));
    assert_eq!(Outcome::Finished, outcome, "{what} finishes");
    got.expect("the run returns")
}

/// Returns each row of `rows`, printed, in order.
pub fn printed(rows: DynRows) -> Vec<String> {
    let form = rows.form().clone();
    rows.map(|r| print(&r.expect("a row"), &form).expect("a row prints"))
        .collect()
}

/// Returns each row of `rows`, printed and sorted.
pub fn sorted(rows: DynRows) -> Vec<String> {
    let mut out = printed(rows);
    out.sort();
    out
}

/// Returns the one result of `text` run in `db`.
pub fn result(db: &Database<SimRuntime>, text: &str) -> DynRows {
    let mut out = db
        .query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
    assert_eq!(1, out.len(), "`{text}` has one result");
    out.remove(0)
}

/// Returns each row of the one result of `text` run in `db`, printed and sorted.
pub fn query(db: &Database<SimRuntime>, text: &str) -> Vec<String> {
    sorted(result(db, text))
}

/// Returns every result of `text` run in `db`.
pub fn results(db: &Database<SimRuntime>, text: &str) -> Vec<DynRows> {
    db.query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"))
}

/// Returns the rows of the last result of `text` run in `db`, printed in order.
pub fn last(db: &Database<SimRuntime>, text: &str) -> Vec<String> {
    printed(results(db, text).pop().expect("the text has a result"))
}

/// Returns the error `text` run in `db` fails with.
pub fn failure(db: &Database<SimRuntime>, text: &str) -> Error {
    match db.query_text(text, &[], &CallOptions::default()) {
        Ok(_) => panic!("`{text}` succeeds"),
        Err(e) => e,
    }
}

pub fn is_invalid(e: &Error) -> bool {
    matches!(
        e,
        Error::Rejected {
            reason: RejectReason::Invalid { .. }
        }
    )
}

/// Returns every result of `text` run in `t`.
pub fn in_txn_results(t: &mut AdHocTransaction<'_, SimRuntime>, text: &str) -> Vec<DynRows> {
    t.query_text(text, &[], &TxnCallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"))
}

/// Returns each row of the one result of `text` run in `t`, printed, in order.
pub fn in_txn(t: &mut AdHocTransaction<'_, SimRuntime>, text: &str) -> Vec<String> {
    let mut out = t
        .query_text(text, &[], &TxnCallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
    assert_eq!(1, out.len(), "`{text}` has one result");
    printed(out.remove(0))
}
