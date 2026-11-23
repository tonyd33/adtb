//! Every entry that runs a program, by every cause of refusal: the answer each gives and the
//! audit rows it leaves.

use adtb::{
    CallOptions, Called, Capability, Database, DynRows, Error, InTxn, TaskOptions, TxnCallOptions,
    TxnOptions, Value,
};
use adtb_lang::types::{Base, Ty};
use adtb_sim::SimRuntime;
use adtb_simkit::api::{printed, result};
use adtb_testkit::process::once;

use crate::server_files::{Server, finish, tree};

#[derive(Clone, Copy, Debug)]
enum Entry {
    RoleText,
    RoleCall,
    EmbeddedText,
    TxnText,
    EmbeddedCall,
    TxnCall,
}

const ENTRIES: [Entry; 6] = [
    Entry::RoleText,
    Entry::RoleCall,
    Entry::EmbeddedText,
    Entry::TxnText,
    Entry::EmbeddedCall,
    Entry::TxnCall,
];

#[derive(Clone, Copy, Debug)]
enum Cause {
    /// The role holds no privilege of the call.
    Ungranted,
    /// The call reads a file outside `file_roots`.
    Outside,
    /// The call alone holds more than the admission queue's byte bound.
    PastByteBound,
    /// The role loses the privilege of the call while the call waits in the queue.
    Revoked,
}

const CAUSES: [Cause; 4] = [
    Cause::Ungranted,
    Cause::Outside,
    Cause::PastByteBound,
    Cause::Revoked,
];

const SECRET: &str = "out/secret.csv";

const READ: &str = "from r in read_csv(p, Named) select r.name";

const PAST_BOUND: &str =
    "Err(Rejected { reason: StatementLimit { limit: Memory { bytes: 67108864 } } })";

fn denied(detail: &str) -> String {
    format!("Err(Rejected {{ reason: Denied {{ detail: \"{detail}\" }} }})")
}

/// Returns the answer `entry` gives for `cause` and the audit rows, `Denied` then `Call`, it
/// adds, with the owner's `Call` row of the text holding the worker while a call waits; `None`
/// where an in-process entry has no role to lack or lose a privilege.
fn expected(entry: Entry, cause: Cause) -> Option<(String, [Vec<&'static str>; 2])> {
    let outside = denied("file `out/secret.csv` is outside file_roots");
    let role = |answer: String, role: &'static str| (answer, [vec![role], Vec::new()]);
    let queued = |answer: String, role: &'static str| (answer, [vec![role], vec!["owner"]]);
    let unrecorded = |answer: &str| (answer.to_owned(), [Vec::new(), Vec::new()]);
    Some(match (entry, cause) {
        (Entry::RoleText, Cause::Ungranted) => role(denied("role `eve` lacks `adhoc`"), "eve"),
        (Entry::RoleCall, Cause::Ungranted) => {
            role(denied("role `web` may not make this call"), "web")
        }
        (Entry::RoleText, Cause::Outside) => role(outside, "owner"),
        (Entry::RoleCall, Cause::Outside) => role(outside, "web"),
        (Entry::RoleText, Cause::Revoked) => queued(denied("role `ann` lacks `adhoc`"), "ann"),
        (Entry::RoleCall, Cause::Revoked) => {
            queued(denied("role `web` may not make this call"), "web")
        }
        (_, Cause::PastByteBound) => unrecorded(PAST_BOUND),
        (_, Cause::Ungranted | Cause::Revoked) => return None,
        (Entry::EmbeddedText | Entry::TxnText, Cause::Outside) => {
            unrecorded(r#"Ok(["\"secret\""])"#)
        }
        (Entry::EmbeddedCall | Entry::TxnCall, Cause::Outside) => unrecorded("Ok([\"1\"])"),
    })
}

/// Returns the path argument `cause` passes.
fn path(cause: Cause) -> String {
    match cause {
        Cause::Ungranted | Cause::Revoked => "root/in.csv".to_owned(),
        Cause::Outside => SECRET.to_owned(),
        Cause::PastByteBound => "x".repeat(64 << 20),
    }
}

/// Run the call `entry` makes for `cause` on `s`. Returns its answer.
fn run(s: &Server, rt: &SimRuntime, entry: Entry, cause: Cause) -> Result<Vec<String>, Error> {
    let db = s.db();
    let p = path(cause);
    let params = [("p", Ty::Base(Base::Text), Value::text(&p))];
    let rows = |rows: Vec<DynRows>| rows.into_iter().flat_map(printed).collect();
    let called = |c: Called| match c {
        Called::Rows(r) => printed(r),
        Called::Committed(c) => printed(c.value),
    };
    let authority = db.role_authority();
    match entry {
        Entry::RoleText => {
            let role = match cause {
                Cause::Ungranted => "eve",
                Cause::Revoked => "ann",
                Cause::Outside | Cause::PastByteBound => "owner",
            };
            if matches!(cause, Cause::Revoked) {
                authority.grant_capability("ann", Capability::Adhoc, false)?;
            }
            let session = authority.session(role)?;
            let hold = matches!(cause, Cause::Revoked).then(|| s.hold());
            let task = session.submit_text(READ, &params, TaskOptions::default())?;
            if let Some(hold) = hold {
                authority.revoke_capability("ann", Capability::Adhoc)?;
                s.release(&hold);
            }
            finish(rt, &task)
        }
        Entry::RoleCall => {
            let bound = s.load();
            let revoke = || {
                let ann = authority.session("ann")?;
                authority.revoke(&ann, "revoke call load-1.0 from web")
            };
            if matches!(cause, Cause::Ungranted) {
                revoke()?;
            }
            let hold = matches!(cause, Cause::Revoked).then(|| s.hold());
            let task = bound.submit(1, vec![Value::text(&p)], TaskOptions::default())?;
            if let Some(hold) = hold {
                revoke()?;
                s.release(&hold);
            }
            finish(rt, &task)
        }
        Entry::EmbeddedText => db
            .query_text(READ, &params, &CallOptions::default())
            .map(rows),
        Entry::TxnText => {
            let mut txn = db.begin(&TxnOptions::default())?;
            let got = txn.query_text(READ, &params, &TxnCallOptions::default());
            txn.rollback()?;
            got.map(rows)
        }
        Entry::EmbeddedCall => {
            s.load();
            let reg = db.registered("load-1.0")?;
            reg.call(1, &[Value::text(&p)], &CallOptions::default())
                .map(called)
        }
        Entry::TxnCall => {
            s.load();
            let reg = db.registered("load-1.0")?;
            let mut txn = db.begin(&TxnOptions::default())?;
            let got = txn.call(&reg, 1, &[Value::text(&p)], &TxnCallOptions::default());
            txn.rollback()?;
            got.map(|c| match c {
                InTxn::Rows(r) | InTxn::Pending(r) => printed(r),
            })
        }
    }
}

/// Returns the roles of the rows of the audit's constructor `ctor`, sorted.
fn audit(db: &Database<SimRuntime>, ctor: &str) -> Vec<String> {
    let mut roles = printed(result(
        db,
        &format!("from a in catalog::audit.{ctor} select a.role"),
    ));
    roles.sort();
    roles
}

/// Returns the rows `after` holds past `before`, the role names unquoted.
fn added(before: &[String], after: &[String]) -> Vec<String> {
    let mut left = before.to_vec();
    let mut out = Vec::new();
    for row in after {
        match left.iter().position(|r| r == row) {
            Some(i) => {
                left.remove(i);
            }
            None => out.push(row.trim_matches('"').to_owned()),
        }
    }
    out
}

/// Every entry that runs a program answers each cause of refusal its caller can meet as stated,
/// and each refusal for a lack of privilege made outside the process leaves one `Denied` row and
/// no `Call` row.
#[test]
fn each_entry_answers_each_refusal_with_its_rows() {
    let base = tree("entry-paths");
    let mut reached = 0;
    let mut seed = 40;
    for entry in ENTRIES {
        for cause in CAUSES {
            let Some((answer, rows)) = expected(entry, cause) else {
                continue;
            };
            seed += 1;
            once(seed, |rt| {
                let s = Server::new(rt.clone(), &base, vec!["root".into()]);
                let before = [audit(s.db(), "Denied"), audit(s.db(), "Call")];
                let got = run(&s, &rt, entry, cause);
                let after = [audit(s.db(), "Denied"), audit(s.db(), "Call")];
                let case = format!("{entry:?} {cause:?}");
                assert_eq!(answer, format!("{got:?}"), "{case}");
                let got_rows = [0, 1].map(|i| added(&before[i], &after[i]));
                assert_eq!(
                    rows.map(|r| r.into_iter().map(str::to_owned).collect::<Vec<_>>()),
                    got_rows,
                    "{case}"
                );
            });
            reached += 1;
        }
    }
    assert_eq!(16, reached, "every applicable pair ran");
}
