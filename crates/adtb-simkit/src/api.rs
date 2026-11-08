//! Open and query a `Database` on the simulator from a test.

use std::sync::Arc;
use std::task::{Wake, Waker};

use adtb::runtime::{ThreadId, Threads};
use adtb::{
    AdHocTransaction, CallOptions, Completion, Database, DynRows, Error, Observation, RejectReason,
    RoleSession, TaskOptions, TxnCallOptions, Value,
};
pub use adtb_harness::process::{once, run};
use adtb_lang::textual::print;
use adtb_lang::types::Ty;
use adtb_sim::SimRuntime;

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

struct Unpark {
    rt: SimRuntime,
    thread: ThreadId,
}

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.rt.unpark(self.thread);
    }
}

/// Submit `text` for `session` as a task and drain it, returning the failure of the submission or
/// the call.
pub fn task(rt: &SimRuntime, session: &RoleSession<SimRuntime>, text: &str) -> Result<(), Error> {
    task_rows(rt, session, text).map(drop)
}

/// Submit `text` as [`task`] does. Returns each row as its debug form.
pub fn task_rows(
    rt: &SimRuntime,
    session: &RoleSession<SimRuntime>,
    text: &str,
) -> Result<Vec<String>, Error> {
    task_bound(rt, session, text, &[])
}

/// Submit `text` as [`task_rows`] does, each free name bound by `params`.
pub fn task_bound(
    rt: &SimRuntime,
    session: &RoleSession<SimRuntime>,
    text: &str,
    params: &[(&str, Ty, Value)],
) -> Result<Vec<String>, Error> {
    let task = session.submit_text(text, params, TaskOptions::default())?;
    let waker = Waker::from(Arc::new(Unpark {
        rt: rt.clone(),
        thread: rt.current(),
    }));
    task.supply(usize::MAX);
    let mut rows = Vec::new();
    loop {
        match task.observe(&waker) {
            Observation::Pending => rt.park(),
            Observation::Layout(_) => {}
            Observation::Batch(b) => {
                let batch = b.rows().expect("a task of values");
                rows.extend(batch.iter().map(|v| format!("{v:?}")));
            }
            Observation::End => task.drained(),
            Observation::Done(Completion::Failed(e)) => return Err(e),
            Observation::Done(_) => return Ok(rows),
        }
    }
}

/// Returns the part a dump refused as damaged. Panics on any other outcome.
pub fn damaged_dump_part<T: std::fmt::Debug>(got: Result<T, Error>) -> String {
    match got {
        Err(Error::Rejected {
            reason: RejectReason::DumpDamaged { part },
        }) => part,
        other => panic!("not DumpDamaged: {other:?}"),
    }
}

/// Returns the part a backup refused as damaged. Panics on any other outcome.
pub fn damaged_backup_part<T: std::fmt::Debug>(got: Result<T, Error>) -> String {
    match got {
        Err(Error::Rejected {
            reason: RejectReason::BackupDamaged { part },
        }) => part,
        other => panic!("not BackupDamaged: {other:?}"),
    }
}
