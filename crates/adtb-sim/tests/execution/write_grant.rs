//! Each write form under a statement's memory grant: a search over doublings of the rows it writes
//! finds a count that fits the grant and its double that is refused for it, changing nothing.

use adtb::{CallOptions, Database, Error, Limit, Log, OpenOptions, RejectReason};
use adtb_sim::SimRuntime;
use adtb_simkit::api::query;
use adtb_simkit::open;
use adtb_testkit::process::once;
use adtb_testkit::refused_as;

const SCHEMA: &str = "module app
add module app
add type Seed = { id: Key seeds, n: I64 }
add relation seeds : Seed key id generate sequence
add type Post = { id: Key posts, body: Text }
add relation posts : Post key id generate sequence
add type User = { id: Key users, n: I64 }
add relation users : User key id generate sequence
add type Note = { id: Key notes, owner: Option (Key users) on delete clear, body: Text }
add relation notes : Note key id generate sequence
add type Ticket { id: Key tickets, body: Text }
  = Open { owner: Key users on delete retag Closed {} }
  | Closed
add relation tickets : Ticket key id generate sequence
add type Login = { id: Key logins, user: Key users on delete cascade, body: Text }
add relation logins : Login key id generate sequence
";

/// The grant of each statement of the search.
const GRANT: u64 = 16384;

/// The most rows the search writes; a form still fitting there fails the test.
const MOST: usize = 4096;

/// The rows each statement that fills a relation inserts, few enough to fit the grant.
const CHUNK: usize = 16;

/// A write form: the relation it reads, the row each of its `n` rows starts as, the write, and a
/// count that is `before` while the write has not run and `after(n)` once it has.
struct Form {
    name: &'static str,
    fill: &'static str,
    row: fn(usize) -> String,
    write: &'static str,
    count: &'static str,
    before: fn(usize) -> usize,
    after: fn(usize) -> usize,
}

fn none(_: usize) -> usize {
    0
}

fn all(n: usize) -> usize {
    n
}

const FORMS: &[Form] = &[
    Form {
        name: "insert",
        fill: "seeds",
        row: |k| format!("{{ id: seeds.key({k}), n: {k} }}"),
        write: r#"insert posts from (from s in seeds select { id: posts.key(s.n), body: "post" ++ print(s.n) })"#,
        count: "count(p for p in posts)",
        before: none,
        after: all,
    },
    Form {
        name: "update",
        fill: "posts",
        row: |k| format!(r#"{{ id: posts.key({k}), body: "post{k}" }}"#),
        write: r#"update p in posts set { p with body: p.body ++ "!" }"#,
        count: r#"count(p for p in posts where ends_with(p.body, "!"))"#,
        before: none,
        after: all,
    },
    Form {
        name: "delete",
        fill: "posts",
        row: |k| format!(r#"{{ id: posts.key({k}), body: "post{k}" }}"#),
        write: "delete p in posts",
        count: "count(p for p in posts)",
        before: all,
        after: none,
    },
    Form {
        name: "upsert",
        fill: "posts",
        row: |k| format!(r#"{{ id: posts.key({k}), body: "post{k}" }}"#),
        write: r#"insert posts from (from p in posts select { id: key p, body: "new" })
unless conflict on key
else update old set { old with body: new.body }"#,
        count: r#"count(p for p in posts where p.body == "new")"#,
        before: none,
        after: all,
    },
    Form {
        name: "clear",
        fill: "notes",
        row: |k| {
            format!(
                r#"{{ id: notes.key({k}), owner: Some {{ value: users.key(1) }}, body: "note{k}" }}"#
            )
        },
        write: "delete u in users",
        count: "count(t for t in notes where t.owner == None)",
        before: none,
        after: all,
    },
    Form {
        name: "retag",
        fill: "tickets.Open",
        row: |k| format!(r#"{{ id: tickets.key({k}), owner: users.key(1), body: "ticket{k}" }}"#),
        write: "delete u in users",
        count: "count(t for t in tickets.Closed)",
        before: none,
        after: all,
    },
    Form {
        name: "cascade",
        fill: "logins",
        row: |k| format!(r#"{{ id: logins.key({k}), user: users.key(1), body: "login{k}" }}"#),
        write: "delete u in users",
        count: "count(l for l in logins)",
        before: all,
        after: none,
    },
];

fn run(db: &Database<SimRuntime>, text: &str) -> Result<(), Error> {
    db.query_text(text, &[], &CallOptions::default()).map(drop)
}

fn count(db: &Database<SimRuntime>, form: &Form) -> usize {
    let got = query(db, form.count);
    assert_eq!(
        1,
        got.len(),
        "{}: `{}` gives one row",
        form.name,
        form.count
    );
    got[0].parse().expect("a count")
}

/// Returns a database under the grant whose relation `form` reads holds `n` rows referring to
/// user 1.
fn filled(rt: &SimRuntime, form: &Form, n: usize) -> Database<SimRuntime> {
    let db = open::opened(
        rt,
        &format!("{}-{n}.adt", form.name),
        &Log::new([SCHEMA]),
        &OpenOptions {
            query_memory_bytes: Some(GRANT),
            ..open::options(1)
        },
    );
    run(&db, "insert users { id: users.key(1), n: 1 }").expect("the user is inserted");
    for start in (1..=n).step_by(CHUNK) {
        let rows: Vec<String> = (start..=n.min(start + CHUNK - 1)).map(form.row).collect();
        let fill = format!("insert {} from ([{}])", form.fill, rows.join(", "));
        run(&db, &fill).unwrap_or_else(|e| panic!("{}: `{fill}` runs: {e:?}", form.name));
    }
    db
}

/// Returns whether `form` writing `n` rows fits the grant, checking that a refusal is for the
/// grant and changes nothing and that a write that fits writes every row.
fn fits(form: &Form, n: usize) -> bool {
    once(n as u64, |rt| {
        let db = filled(&rt, form, n);
        assert_eq!(
            (form.before)(n),
            count(&db, form),
            "{}: {n} rows",
            form.name
        );
        if let Err(e) = run(&db, form.write) {
            let refused = Error::Rejected {
                reason: RejectReason::StatementLimit {
                    limit: Limit::Memory {
                        bytes: GRANT as i64,
                    },
                },
            };
            let reason = format!("the statement exceeded its memory limit of {GRANT} bytes");
            refused_as(Err::<(), _>(e), &refused, &reason);
            assert_eq!(
                (form.before)(n),
                count(&db, form),
                "{}: the refused write of {n} rows changes nothing",
                form.name
            );
            return false;
        }
        assert_eq!((form.after)(n), count(&db, form), "{}: {n} rows", form.name);
        true
    })
}

#[test]
fn each_write_form_fits_some_rows_in_a_grant_and_is_refused_for_it_at_their_double() {
    for form in FORMS {
        let mut n = 1;
        assert!(
            fits(form, n),
            "{}: one row fits a grant of {GRANT}",
            form.name
        );
        while fits(form, 2 * n) {
            n *= 2;
            assert!(
                n < MOST,
                "{}: {MOST} rows fit a grant of {GRANT}",
                form.name
            );
        }
    }
}
