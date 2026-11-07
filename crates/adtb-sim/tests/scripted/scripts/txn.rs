//! Named interactive transactions and asserted principals.

use std::collections::BTreeMap;

use adtb::{
    AdHocTransaction, CallOptions, Database, Error, InTxn, TxnCallOptions, TxnOptions, Value,
};
use adtb_lang::textual::{self, Form};
use adtb_lang::types::Ty;
use adtb_sim::SimRuntime;

use super::read::{Head, Step};
use super::role::{Sent, collected};
use super::session::{Differs, compare, refused};

/// The transactions a script holds open, by name.
pub(super) type Txns<'a> = BTreeMap<String, AdHocTransaction<'a, SimRuntime>>;

/// Returns the principal the step's `assert` prefix states, a value of the schema's `Principal`.
pub(super) fn asserted(db: &Database<SimRuntime>, step: &Step) -> Result<Option<Value>, Differs> {
    let Some(printed) = &step.assert else {
        return Ok(None);
    };
    let schema = db.schema();
    let ty = Ty::Data {
        def: schema.principal,
        args: Vec::new(),
        ctors: None,
    };
    textual::parse(printed, &Form::of(schema, &ty))
        .map(Some)
        .map_err(|e| Differs::at(step, format!("`{printed}` is no principal: {e}")))
}

/// Returns the transaction the step's `in` prefix names.
pub(super) fn named<'t, 'a>(
    txns: &'t mut Txns<'a>,
    step: &Step,
) -> Result<Option<&'t mut AdHocTransaction<'a, SimRuntime>>, Differs> {
    match &step.txn {
        None => Ok(None),
        Some(name) => txns
            .get_mut(name)
            .map(Some)
            .ok_or_else(|| Differs::at(step, format!("no transaction `{name}` is open"))),
    }
}

/// Returns how the step's body ended, sent as one call in process for the asserted principal
/// or as one call of the transaction the step names.
pub(super) fn texted(
    db: &Database<SimRuntime>,
    txns: &mut Txns<'_>,
    step: &Step,
) -> Result<Sent, Differs> {
    let got = match named(txns, step)? {
        Some(t) => t.query_text(&step.body, &[], &TxnCallOptions::default()),
        None => {
            let opts = CallOptions {
                assert: asserted(db, step)?,
                ..CallOptions::default()
            };
            db.query_text(&step.body, &[], &opts)
        }
    };
    Ok(match got {
        Err(e) => Sent::Refused(e),
        Ok(results) => match results.into_iter().map(collected).collect() {
            Err(e) => Sent::Failed(e),
            Ok(results) => Sent::Done(results),
        },
    })
}

/// Run a `begin`, `commit` or `rollback` step.
pub(super) fn ended<'a>(
    db: &'a Database<SimRuntime>,
    txns: &mut Txns<'a>,
    step: &Step,
    positions: &mut BTreeMap<String, String>,
) -> Result<(), Differs> {
    let (got, want): (Result<(), Error>, _) = match &step.head {
        Head::Begin {
            name,
            read_only,
            deferrable,
            error,
        } => {
            if txns.contains_key(name) {
                return Err(Differs::at(step, format!("transaction `{name}` is open")));
            }
            let opts = TxnOptions {
                read_only: *read_only,
                deferrable: *deferrable,
                assert: asserted(db, step)?,
                ..TxnOptions::default()
            };
            let got = db.begin(&opts).map(|t| {
                txns.insert(name.clone(), t);
            });
            (got, error)
        }
        Head::Commit {
            name,
            position,
            error,
        } => {
            if let Some(at) = position.as_ref().filter(|at| positions.contains_key(*at)) {
                return Err(Differs::at(
                    step,
                    format!("position `{at}` is bound already"),
                ));
            }
            let got = taken(txns, step, name)?.commit(None).map(|p| {
                if let Some(at) = position {
                    let shown = format!("{{ epoch: {}, lsn: {} }}", p.epoch, p.lsn);
                    positions.insert(at.clone(), shown);
                }
            });
            (got, error)
        }
        Head::Rollback(name) => (taken(txns, step, name)?.rollback(), &None),
        _ => unreachable!("only begin, commit and rollback end"),
    };
    compare(
        step,
        want.as_deref(),
        got.map(|()| Vec::new()).map_err(|e| refused(&e)),
    )
}

fn taken<'a>(
    txns: &mut Txns<'a>,
    step: &Step,
    name: &str,
) -> Result<AdHocTransaction<'a, SimRuntime>, Differs> {
    txns.remove(name)
        .ok_or_else(|| Differs::at(step, format!("no transaction `{name}` is open")))
}

/// Returns the form and elements of the rows a call in a transaction returned, and whether they
/// are the value of a registered transaction committing with it.
pub(super) fn pending(got: InTxn) -> Result<(bool, (Form, Vec<Value>)), Error> {
    match got {
        InTxn::Rows(rows) => collected(rows).map(|r| (false, r)),
        InTxn::Pending(rows) => collected(rows).map(|r| (true, r)),
    }
}
