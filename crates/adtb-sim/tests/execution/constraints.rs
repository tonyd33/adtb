#[path = "constraints/names.rs"]
mod names;

use std::collections::BTreeSet;

use adtb::Value as Val;
use adtb::{CallOptions, Database, Error, RejectReason, TxnCallOptions, TxnOptions};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_simkit::load;

/// Run `body` over a new database of the declarations `src` in a simulation of `seed`, and
/// return what it returns.
fn with_engine<T>(seed: u64, src: &str, body: impl FnOnce(&Database<SimRuntime>) -> T) -> T {
    let mut got = None;
    let mut sim = Sim::new(seed, Config::default());
    let outcome = sim.run(|rt| got = Some(body(&load::open(&rt, src))));
    assert_eq!(Outcome::Finished, outcome, "seed {seed}");
    got.unwrap_or_else(|| panic!("seed {seed}: the run gave nothing"))
}

/// Returns the rows `text` reads in a read-only transaction of its own.
pub(crate) fn read(db: &Database<SimRuntime>, text: &str) -> BTreeSet<Val> {
    let mut rows = db
        .query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` runs: {e:?}"));
    let [rows] = rows.as_mut_slice() else {
        panic!("`{text}` holds one statement")
    };
    rows.map(|r| r.expect("a row")).collect()
}

/// Run `text` as a transaction of its own. Panics, naming `what`, when it fails.
fn run(db: &Database<SimRuntime>, text: &str, what: &str) {
    db.query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("{what} fails: {e:?}"));
}

const PEOPLE: &str = r"
type User = { id: Key users, name: Text }
relation users : User key id generate sequence index (name)
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text }
  | Google    { gid: Text }
relation identities : Identity key id generate sequence index (user) unique (gid) in Google
type Ban = { gid: Text }
relation bans : Ban key gid
";

const HAS_ONE: &str = "constraint has_identity =
  from u in users
  where not exists i in identities where i.user == key u
  select u
";

const ROSTER: &str = r"
type Membership = { id: Key memberships, user: Text, org: Text }
relation memberships : Membership key id generate sequence unique (user, org)
type Assignment = { id: Key assignments, user: Text, org: Text, task: Text }
relation assignments : Assignment key id generate sequence index (user, org)
constraint assigned_member =
  from a in assignments
  where not exists m in memberships where m.user == a.user and m.org == a.org
  select a
";

/// Runs `a` and `b` in two transactions in the order `order` gives, a sequence over `'a'` and
/// `'b'` for a statement and `'A'` and `'B'` for a commit. Returns how each ended.
fn interleave(
    db: &Database<SimRuntime>,
    a: &str,
    b: &str,
    order: &str,
) -> (Result<(), Error>, Result<(), Error>) {
    let mut ends: [Result<(), Error>; 2] = [Ok(()), Ok(())];
    let mut open = [
        Some(db.begin(&TxnOptions::default()).expect("a transaction")),
        Some(db.begin(&TxnOptions::default()).expect("a transaction")),
    ];
    for step in order.chars() {
        let i = usize::from(step.eq_ignore_ascii_case(&'b'));
        if ends[i].is_err() {
            continue;
        }
        let mut tx = open[i].take().expect("an open transaction");
        if step.is_ascii_lowercase() {
            let text = if i == 0 { a } else { b };
            match tx.query_text(text, &[], &TxnCallOptions::default()) {
                Ok(_) => open[i] = Some(tx),
                Err(e) => {
                    ends[i] = Err(e);
                    tx.rollback().expect("a rollback");
                }
            }
        } else if let Err(e) = tx.commit(None) {
            ends[i] = Err(e);
        }
    }
    let [ra, rb] = ends;
    (ra, rb)
}

/// Returns whether `e` is a serialization failure or a violation of a constraint.
fn serial_or_violated(e: &Error) -> bool {
    e.is_serialization()
        || matches!(
            e,
            Error::Rejected {
                reason: RejectReason::Violated { .. }
            }
        )
}

/// Asserts that after `setup`, two transactions running `a` and `b`, each of which keeps the
/// constraint whose query is `denial` alone, never both commit in any order of their
/// statements and commits, that a failure is a serialization failure, and that the constraint
/// holds afterwards.
fn serial(name: &str, src: &str, setup: &str, a: &str, b: &str, denial: &str) {
    for order in ["abAB", "abBA", "aAbB", "baAB", "baBA", "bBaA"] {
        for seed in 0..2 {
            let (ra, rb, left) = with_engine(seed, src, |db| {
                run(db, setup, &format!("{name}, seed {seed}: the setup"));
                let (ra, rb) = interleave(db, a, b, order);
                (ra, rb, read(db, denial))
            });
            let ctx = format!("{name}, order {order}, seed {seed}: {ra:?} {rb:?}");
            if order.starts_with("aA") || order.starts_with("bB") {
                assert!(ra.is_ok() != rb.is_ok(), "{ctx}: not exactly one commits");
            } else {
                assert!(ra.is_err() || rb.is_err(), "{ctx}: both commit");
            }
            for e in [&ra, &rb].into_iter().filter_map(|r| r.as_ref().err()) {
                assert!(
                    serial_or_violated(e),
                    "{ctx}: a failure other than serialization or the constraint"
                );
            }
            assert_eq!(BTreeSet::new(), left, "{ctx}: the constraint is violated");
        }
    }
}

#[test]
fn concurrent_deletes_of_two_identities_cannot_jointly_violate_at_least_one() {
    serial(
        "has_identity",
        &format!("{PEOPLE}{HAS_ONE}"),
        r#"transact
  insert users { id: users.key(1), name: "ann" }
  insert identities.Google { id: identities.key(1), user: users.key(1), gid: "g1" }
  insert identities.Google { id: identities.key(2), user: users.key(1), gid: "g2" }"#,
        "delete i in identities where key i == identities.key(1)",
        "delete i in identities where key i == identities.key(2)",
        "from u in users where not exists i in identities where i.user == key u select u",
    );
}

#[test]
fn concurrent_inserts_cannot_jointly_pass_a_count() {
    let src = format!(
        "{PEOPLE}
constraint identity_limit =
  from u in users
  where count(i for i in identities where i.user == key u) > 2
  select u
"
    );
    serial(
        "identity_limit",
        &src,
        r#"transact
  insert users { id: users.key(1), name: "ann" }
  insert identities.Google { id: identities.key(1), user: users.key(1), gid: "g1" }"#,
        r#"insert identities.Google { id: identities.key(2), user: users.key(1), gid: "g2" }"#,
        r#"insert identities.Google { id: identities.key(3), user: users.key(1), gid: "g3" }"#,
        "from u in users where count(i for i in identities where i.user == key u) > 2 select u",
    );
}

#[test]
fn a_concurrent_delete_and_insert_cannot_jointly_violate_an_inclusion() {
    serial(
        "assigned_member",
        ROSTER,
        r#"insert memberships { id: memberships.key(1), user: "u", org: "o1" }"#,
        r#"delete m in memberships where m.user == "u""#,
        r#"insert assignments { id: assignments.key(1), user: "u", org: "o1", task: "t" }"#,
        "from a in assignments where not exists m in memberships where m.user == a.user and m.org == a.org select a",
    );
}

const OWNED: &str = r"
type User = { id: Key users, name: Text }
relation users : User key id generate sequence
type Account = { id: Key accounts, owner: Key users }
relation accounts : Account key id generate sequence
type Note = { id: Key notes, about: Key users on delete cascade }
relation notes : Note key id generate sequence
";

#[test]
fn deleting_a_referenced_row_reads_its_referrers_through_the_implied_index() {
    let setup = r#"transact
  insert users { id: users.key(1), name: "ann" }
  insert users { id: users.key(2), name: "bo" }
  insert users { id: users.key(3), name: "cy" }
  insert accounts { id: accounts.key(1), owner: users.key(1) }
  insert accounts { id: accounts.key(3), owner: users.key(3) }
  insert notes { id: notes.key(2), about: users.key(2) }"#;
    let a = "delete u in users where key u == users.key(2)";
    let b = "transact
  insert accounts from (from u in users where key u == users.key(2)
                        select { id: accounts.key(10), owner: users.key(1) })
  insert notes { id: notes.key(10), about: users.key(3) }";
    for order in ["abAB", "abBA", "aAbB", "baAB", "baBA", "bBaA"] {
        for seed in 0..2 {
            let (ra, rb, notes) = with_engine(seed, OWNED, |db| {
                run(db, setup, &format!("seed {seed}: the setup"));
                let (ra, rb) = interleave(db, a, b, order);
                (ra, rb, read(db, "from n in notes select n.id"))
            });
            let ctx = format!("order {order}, seed {seed}: {ra:?} {rb:?}");
            assert!(ra.is_ok() && rb.is_ok(), "{ctx}: a probe conflicts");
            assert_eq!(BTreeSet::from([Val::Int(10)]), notes, "{ctx}: the cascade");
        }
    }
}
