//! Steps through a session of a role: ad hoc text sent as a task, calls through a binding,
//! grants and revokes, and the host functions of an open.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::task::{Wake, Waker};

use adtb::batch::{Batch, BatchBuilder, Schema, Type};
use adtb::key::Scalar;
use adtb::runtime::{ThreadId, Threads};
use adtb::{
    CallOptions, Called, Completion, Database, Determinism, DynRows, Error, Observation,
    OpenOptions, Pins, Registered, RoleBinding, Task, TaskOptions, TxnCallOptions, Value,
};
use adtb_lang::textual::Form;
use adtb_sim::SimRuntime;

use super::read::{Head, Step};
use super::session::{Answer, Differs, arguments, call_rows, compare, refused, rejected};
use super::txn::{Txns, asserted, named, pending};

/// The schema owner's role, which a `grant`, `revoke` or `register` step with no `as` prefix
/// acts as.
pub(super) const OWNER: &str = "owner";

/// A handle a `call` with no ordinal kept: the registered handle of the schema owner, or a
/// binding of a role.
pub(super) enum Handle {
    Owner(Registered<SimRuntime>),
    Role(RoleBinding<SimRuntime>),
}

/// The handles kept, by the step's role and the name bound.
pub(super) type Handles = BTreeMap<(Option<String>, String), Handle>;

/// Returns how the call of `step` ended, the arguments the queries of its body evaluate to. With
/// no ordinal, bind the name and keep the handle; else call through the handle kept for the name
/// when there is one. A step with a role binds through a session of it.
pub(super) fn called(
    rt: &SimRuntime,
    db: &Database<SimRuntime>,
    handles: &mut Handles,
    txns: &mut Txns<'_>,
    step: &Step,
) -> Result<Answer, Differs> {
    let Head::Call {
        name,
        ordinal,
        committed,
        ..
    } = &step.head
    else {
        unreachable!("only a call step calls")
    };
    let args = arguments(db, step)?;
    let authority = db.role_authority();
    let bind = || match &step.role {
        None => db.registered(name).map(Handle::Owner),
        Some(role) => authority
            .session(role)
            .and_then(|s| authority.bind(&s, name, &Pins::default()))
            .map(Handle::Role),
    };
    let key = (step.role.clone(), name.clone());
    let Some(ordinal) = *ordinal else {
        return Ok(match bind() {
            Ok(handle) => {
                handles.insert(key, handle);
                Answer::Bound
            }
            Err(e) => Answer::Refused(refused(&e)),
        });
    };
    let fresh;
    let handle = match handles.get(&key) {
        Some(kept) => kept,
        None => match bind() {
            Ok(handle) => {
                fresh = handle;
                &fresh
            }
            Err(e) => return Ok(Answer::Refused(refused(&e))),
        },
    };
    let opts = asserting(db, step)?;
    let got = match (handle, named(txns, step)?) {
        (Handle::Owner(reg), Some(t)) => t
            .call(reg, ordinal, &args, &TxnCallOptions::default())
            .and_then(pending)
            .map(|(committed, r)| (committed, vec![r])),
        (Handle::Role(_), Some(_)) => unreachable!("a role calls in no transaction"),
        (Handle::Owner(reg), None) => reg.call(ordinal, &args, &opts.call).and_then(|c| match c {
            Called::Rows(rows) => collected(rows).map(|r| (false, vec![r])),
            Called::Committed(c) => collected(c.value).map(|r| (true, vec![r])),
        }),
        (Handle::Role(binding), None) => match binding.submit(ordinal, args, opts) {
            Err(e) => Err(e),
            Ok(task) => match drain(rt, &task) {
                Err(e) => return Ok(Answer::Failed(refused(&e))),
                Ok((results, c)) => Ok((matches!(c, Completion::Committed { .. }), results)),
            },
        },
    };
    match got {
        Err(e) => Ok(Answer::Refused(refused(&e))),
        Ok((false, _)) if *committed => Err(Differs::at(step, "the call commits nothing")),
        Ok((true, _)) if !*committed => Err(Differs::at(step, "the call commits")),
        Ok((_, mut results)) => match (results.pop(), results.is_empty()) {
            (Some((form, rows)), true) => Ok(Answer::Rows(form, rows)),
            _ => Err(Differs::at(step, "the call has other than one result")),
        },
    }
}

/// Returns the form and elements of `rows`.
pub(super) fn collected(rows: DynRows) -> Result<(Form, Vec<Value>), Error> {
    let form = rows.form().clone();
    Ok((form, rows.collect::<Result<_, _>>()?))
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

fn asserting(db: &Database<SimRuntime>, step: &Step) -> Result<TaskOptions, Differs> {
    Ok(TaskOptions {
        call: CallOptions {
            assert: asserted(db, step)?,
            ..CallOptions::default()
        },
        ..TaskOptions::default()
    })
}

/// The form and elements of each result of a task, in order.
pub(super) type Results = Vec<(Form, Vec<Value>)>;

/// Returns each result of `task` and how it completed, or its failure.
fn drain(rt: &SimRuntime, task: &Task<SimRuntime>) -> Result<(Results, Completion), Error> {
    let waker = Waker::from(Arc::new(Unpark {
        rt: rt.clone(),
        thread: rt.current(),
    }));
    task.supply(usize::MAX);
    let mut results: Results = Vec::new();
    loop {
        match task.observe(&waker) {
            Observation::Pending => rt.park(),
            Observation::Layout(l) => results.push((l.form().clone(), Vec::new())),
            Observation::Batch(b) => {
                let rows = b.rows().expect("a task submitted for rows has rows");
                let result = results.last_mut().expect("a layout precedes each batch");
                result.1.extend(rows.iter().cloned());
            }
            Observation::End => task.drained(),
            Observation::Done(Completion::Failed(e)) => return Err(e),
            Observation::Done(c) => return Ok((results, c)),
        }
    }
}

/// How ad hoc text sent as one call ended.
pub(super) enum Sent {
    /// Refused at the submit, the session's included, or by the call in process.
    Refused(Error),
    /// Admitted, and failed when it completed.
    Failed(Error),
    Done(Results),
}

/// Returns how `text` sent as one ad hoc call of a session of `role` ended.
pub(super) fn sent(
    rt: &SimRuntime,
    db: &Database<SimRuntime>,
    step: &Step,
) -> Result<Sent, Differs> {
    let role = step.role.as_deref().expect("a step sent as a role");
    let opts = asserting(db, step)?;
    let submitted = db
        .role_authority()
        .session(role)
        .and_then(|s| s.submit_text(&step.body, &[], opts));
    Ok(match submitted {
        Err(e) => Sent::Refused(e),
        Ok(task) => match drain(rt, &task) {
            Err(e) => Sent::Failed(e),
            Ok((results, _)) => Sent::Done(results),
        },
    })
}

/// Supply host function `name`, version `1.0` and deterministic, answering its own name as text
/// for each row.
pub(super) fn host(options: &mut OpenOptions, name: &str) {
    let reply = name.to_owned();
    let answer = move |b: &Batch| {
        let mut out = BatchBuilder::new(Schema::new(vec![Type::Scalar(Scalar::Text)]));
        for _ in 0..b.len() {
            out.push_row(&[adtb::key::Value::Text(reply.clone())]);
        }
        Ok(out.finish())
    };
    options
        .host_fns
        .register(name, "1.0", Determinism::Deterministic, answer);
}

/// Give or revoke each statement of the step's body in order as its role, the last refusal
/// compared with `want`.
pub(super) fn grant(
    db: &Database<SimRuntime>,
    step: &Step,
    revoke: bool,
    want: Option<&str>,
) -> Result<(), Differs> {
    let authority = db.role_authority();
    let role = step.role.as_deref().unwrap_or(OWNER);
    let lines: Vec<&str> = step.body.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let done = authority.session(role).and_then(|s| match revoke {
            true => authority.revoke(&s, line),
            false => authority.grant(&s, line),
        });
        match done {
            Err(e) if i + 1 == lines.len() => {
                return compare(step, want, Err(refused(&e)));
            }
            Err(e) => {
                let shown = format!("`{line}` is refused: {}", refused(&e));
                return Err(Differs::at(step, shown));
            }
            Ok(()) => {}
        }
    }
    compare(step, want, Ok(Vec::new()))
}

/// Compare how the body of a `run`, `query`, `error` or `failed` step sent as one call ended.
pub(super) fn sent_step(step: &Step, got: Sent) -> Result<(), Differs> {
    match (&step.head, got) {
        (Head::Run, Sent::Done(_)) => Ok(()),
        (Head::Run | Head::Query { .. }, Sent::Refused(e)) => {
            Err(Differs::at(step, format!("refused with {}", refused(&e))))
        }
        (Head::Run | Head::Query { .. }, Sent::Failed(e)) => Err(Differs::at(
            step,
            format!("fails at completion with {}", refused(&e)),
        )),
        (Head::Query { rowsort }, Sent::Done(mut results)) => {
            match (results.pop(), results.is_empty()) {
                (Some((form, rows)), true) => call_rows(step, &form, rows, *rowsort),
                _ => Err(Differs::at(step, "a query holds one statement")),
            }
        }
        (Head::Error(reason), got) => {
            let want = rejected(reason);
            match got {
                Sent::Refused(e) if refused(&e) == want => Ok(()),
                Sent::Refused(e) => {
                    Err(Differs::at(step, "refused otherwise").values(want, refused(&e)))
                }
                Sent::Failed(e) => Err(Differs::at(step, "admitted, and fails at completion")
                    .values(want, refused(&e))),
                Sent::Done(_) => Err(Differs::at(step, "succeeds").values(want, "success".into())),
            }
        }
        (Head::Failed(reason), got) => {
            let want = rejected(reason);
            match got {
                Sent::Failed(e) if refused(&e) == want => Ok(()),
                Sent::Failed(e) => {
                    Err(Differs::at(step, "fails otherwise").values(want, refused(&e)))
                }
                Sent::Refused(e) => {
                    Err(Differs::at(step, "refused at the submit").values(want, refused(&e)))
                }
                Sent::Done(_) => Err(Differs::at(step, "succeeds").values(want, "success".into())),
            }
        }
        _ => unreachable!("only run, query, error and failed steps are sent"),
    }
}
