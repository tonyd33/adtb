use std::ops::Range;
mod plan {
    pub use adtb::plan::*;
}

#[path = "../../adtb/src/plan/tests/fixture.rs"]
#[expect(dead_code, reason = "this binary uses part of the planner fixture")]
mod planfix;

#[expect(
    dead_code,
    reason = "this binary binds databases for writes and loads none for reads"
)]
mod support;

#[expect(dead_code, reason = "this binary uses part of the device helpers")]
mod common;

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use adtb::exec::{self, Budget, Scheduler, Val};
use adtb::mvcc::Limits;
use adtb::plan::Catalog;
use adtb::write::{self, Bindings, Database, Rejection as WRejection, Rows, Setup};
use adtb_lang::check::check_adhoc;
use adtb_lang::kernel::{Terms, Tm, Var};
use adtb_lang::norm::{self, NormalProgram};
use adtb_lang::parse::{Options, parse_adhoc};
use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;
use adtb_reference::{Abort, Rejection, Value};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_testkit::fixture::Fixture;
use adtb_testkit::uuid;
use adtb_testkit::writes::fixture;

use support::harness::finished;
use support::seeds::share;
use support::{Db, World, to_ref, to_val};

/// One statement of a case: its text, its parameters as name, type and value, and the clock it
/// runs at.
struct Step {
    text: String,
    params: Vec<(&'static str, &'static str, Value)>,
    clock: i64,
}

fn step(text: &str) -> Step {
    Step {
        text: text.to_string(),
        params: Vec::new(),
        clock: 0,
    }
}

fn with(text: &str, params: Vec<(&'static str, &'static str, Value)>) -> Step {
    Step {
        text: text.to_string(),
        params,
        clock: 0,
    }
}

fn at(clock: i64, text: &str) -> Step {
    Step {
        text: text.to_string(),
        params: Vec::new(),
        clock,
    }
}

/// A sequence of statements over one database.
struct Case {
    name: &'static str,
    fixture: fn() -> Fixture,
    steps: Vec<Step>,
}

/// What a statement did, with every value erased of the reference's key wrappers and names.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Got {
    result: Val,
    inserted: Rows,
    deleted: Rows,
}

/// Returns reference value `v` as the executor holds a value, whatever its type.
fn erase(v: &Value) -> Val {
    match v {
        Value::Int(x) => Val::Int(*x),
        Value::Decimal(d, _) => Val::Decimal(*d),
        Value::F32(x) => Val::F32(exec::F32::new(*x)),
        Value::F64(x) => Val::F64(exec::F64::new(*x)),
        Value::Text(t) => Val::text(t),
        Value::Bytes(b) => Val::Bytes(Arc::from(b.as_slice())),
        Value::SortKey(b) => Val::SortKey(Arc::from(b.as_slice())),
        Value::Instant(x) => Val::Instant(*x),
        Value::Date(x) => Val::Date(*x as i32),
        Value::Time(x) => Val::Time(*x),
        Value::DateTime(x) => Val::DateTime(*x),
        Value::Zoned(l, z) => Val::Zoned {
            local: *l,
            zone: Arc::from(z.as_str()),
        },
        Value::Duration(x) => Val::Duration(*x),
        Value::Period { months, days } => Val::Period {
            months: *months,
            days: *days,
        },
        Value::Ctor(c, fs) => Val::ctor(*c, fs.iter().map(|(_, x)| erase(x)).collect()),
        Value::Record(fs) => Val::record(fs.iter().map(|(_, x)| erase(x)).collect()),
        Value::Key(x) => erase(x),
        Value::Set(s) => Val::set(s.iter().map(erase).collect()),
        Value::List(xs) => Val::List(Arc::from(xs.iter().map(erase).collect::<Vec<_>>())),
        Value::Fn(_) => panic!("a statement's outcome holds no function"),
    }
}

fn erase_db(db: &adtb_reference::Database) -> Rows {
    db.iter()
        .filter(|(_, rows)| !rows.is_empty())
        .map(|(k, rows)| (*k, rows.iter().map(erase).collect()))
        .collect()
}

/// An abort as the two sides are compared: what it is, and the rows it reports.
type Failed = (String, BTreeSet<Val>);

fn plain(s: String) -> Failed {
    (s, BTreeSet::new())
}

fn reference_error(a: &Abort) -> Failed {
    match a {
        Abort::Fault(f) => plain(format!("Fault {}", f.name())),
        Abort::Limit(_) => plain("Limit".into()),
        Abort::Reject(r) => match r {
            Rejection::KeyViolation { relation, key } => {
                plain(format!("KeyViolation {relation} {key}"))
            }
            Rejection::Violated { constraint, report } => (
                format!("Violated {constraint}"),
                report.iter().map(erase).collect(),
            ),
            Rejection::KeyChanged { relation } => plain(format!("KeyChanged {relation}")),
            Rejection::NotOpen { ctor, stage } => plain(format!("NotOpen {ctor} {stage:?}")),
            Rejection::UserError { message } => plain(format!("UserError {message}")),
            Rejection::RowTooLarge {
                relation,
                ctor,
                size,
                limit,
            } => plain(format!("RowTooLarge {relation} {ctor} {size} {limit}")),
            other => plain(format!("{other:?}")),
        },
        other => plain(format!("{other:?}")),
    }
}

fn engine_error(schema: &Schema, e: &write::Error) -> Failed {
    match e {
        write::Error::Fault(f) => plain(format!("Fault {}", f.name())),
        write::Error::StatementLimit(_) => plain("Limit".into()),
        write::Error::Rejected(r) => match r {
            WRejection::KeyViolation { relation, key } => {
                let ty = Ty::Key {
                    rel: *relation,
                    ctors: None,
                };
                let printed =
                    adtb_reference::textual::print(schema, &to_ref(schema, &ty, key), &ty)
                        .expect("a key prints");
                plain(format!(
                    "KeyViolation {} {printed}",
                    schema.relation(*relation).name
                ))
            }
            WRejection::Violated { constraint, report } => {
                (format!("Violated {constraint}"), report.clone())
            }
            WRejection::KeyChanged { relation } => {
                plain(format!("KeyChanged {}", schema.relation(*relation).name))
            }
            WRejection::NotOpen { ctor, stage } => plain(format!("NotOpen {ctor} {stage:?}")),
            WRejection::UserError { message } => plain(format!("UserError {message}")),
            WRejection::BadInput {
                source,
                line,
                detail,
            } => plain(format!(
                "BadInput {{ source: {source:?}, line: {line}, detail: {detail:?} }}"
            )),
            WRejection::RowTooLarge {
                relation,
                ctor,
                size,
                limit,
            } => {
                let rel = schema.relation(*relation);
                let ctor = &schema.ty(rel.elem_def).ctors()[*ctor as usize].name;
                plain(format!("RowTooLarge {} {ctor} {size} {limit}", rel.name))
            }
        },
        other => plain(format!("{other:?}")),
    }
}

/// A statement ready for the engine: its normal form, and its parameters' values.
struct Prepared {
    terms: Terms,
    item: Tm,
    params: Vec<(Var, Val)>,
    clock: i64,
}

fn prepare(f: &Fixture, s: &Step) -> Prepared {
    let items = parse_adhoc(&s.text, Options::default())
        .ok()
        .unwrap_or_else(|_| panic!("`{}` does not parse", s.text));
    let tys: BTreeMap<String, Ty> = s
        .params
        .iter()
        .map(|(n, t, _)| (n.to_string(), f.ty(t)))
        .collect();
    let adhoc = check_adhoc(&f.schema, &f.program, &items, &tys)
        .unwrap_or_else(|ds| panic!("`{}` does not check: {ds:?}", s.text));
    let normal = norm::adhoc(&f.schema, &f.program, &adhoc)
        .unwrap_or_else(|e| panic!("`{}` has no normal form: {e:?}", s.text));
    let params = s
        .params
        .iter()
        .map(|(n, _, v)| {
            let var = normal.params[*n];
            (var, to_val(&f.schema, &normal.terms.var_info(var).ty, v))
        })
        .collect();
    Prepared {
        item: normal.items[0],
        terms: normal.terms,
        params,
        clock: s.clock,
    }
}

type Outcomes = (Vec<Result<Got, Failed>>, Rows);

fn reference(case: &Case) -> Outcomes {
    let mut f = (case.fixture)();
    let mut out = Vec::new();
    for s in &case.steps {
        f.supply.clock = s.clock;
        let got = match f.run_with(&s.text, &s.params) {
            Ok(o) => Ok(Got {
                result: erase(&o.result),
                inserted: erase_db(&o.change.inserted),
                deleted: erase_db(&o.change.deleted),
            }),
            Err(a) => Err(reference_error(&a)),
        };
        out.push(got);
    }
    (out, erase_db(&f.db))
}

/// How the engine runs a case: on `workers` morsel workers, or on the calling thread when 0;
/// each statement its own transaction, or with `one` every statement in one transaction.
#[derive(Clone, Copy, Debug)]
struct Mode {
    workers: usize,
    one: bool,
}

fn engine(seed: u64, case: &Case, mode: Mode) -> Outcomes {
    let f = (case.fixture)();
    let catalog = Arc::new(planfix::catalog(&f.schema));
    let normal: Arc<NormalProgram> =
        Arc::new(norm::program(&f.schema, &f.program).expect("the schema normalises"));
    let prepared: Vec<Prepared> = case.steps.iter().map(|s| prepare(&f, s)).collect();
    let schema = Arc::new(f.schema.clone());
    let mut got = None;
    let mut sim = Sim::new(seed, Config::default());
    let result = sim.run(|rt| {
        got = Some(run_engine(
            &rt, &f, &schema, &normal, &catalog, &prepared, mode,
        ));
    });
    assert_eq!(Outcome::Finished, result, "seed {seed}, case {}", case.name);
    got.unwrap_or_else(|| panic!("seed {seed}, case {}: the run gave no outcomes", case.name))
}

fn run_engine(
    rt: &SimRuntime,
    f: &Fixture,
    schema: &Arc<Schema>,
    normal: &Arc<NormalProgram>,
    catalog: &Arc<Catalog>,
    prepared: &[Prepared],
    mode: Mode,
) -> Outcomes {
    let world = World {
        schema: &f.schema,
        normal,
        catalog,
        db: &f.db,
    };
    let d = Db::load_with(rt, &world, false);
    let pool = (mode.workers > 0).then(|| Scheduler::start(rt.clone(), mode.workers));
    let db = database(rt, d, schema, normal, catalog, pool.clone(), None);
    let mut out = Vec::new();
    let mut open = None;
    for p in prepared {
        let tx = match open.take() {
            Some(tx) => tx,
            None => db.begin().expect("a transaction"),
        };
        let got = db.statement(&tx, &p.terms, p.item, &p.params, p.clock);
        out.push(match &got {
            Ok(o) => Ok(Got {
                result: o.result.clone(),
                inserted: o.inserted.clone(),
                deleted: o.deleted.clone(),
            }),
            Err(e) => Err(engine_error(schema, e)),
        });
        match got {
            Err(e) if e.ends_transaction() => db.abort(tx).expect("an abort"),
            _ if mode.one => open = Some(tx),
            Ok(_) => {
                db.commit(tx).expect("a commit");
            }
            Err(_) => db.abort(tx).expect("an abort"),
        }
    }
    if let Some(tx) = open {
        db.commit(tx).expect("a commit");
    }
    let tx = db.begin().expect("a transaction");
    let rows = db.rows(&tx).expect("the rows");
    db.abort(tx).expect("an abort");
    if let Some(p) = pool {
        p.shutdown();
    }
    (out, rows)
}

fn database(
    rt: &SimRuntime,
    d: Db,
    schema: &Arc<Schema>,
    normal: &Arc<NormalProgram>,
    catalog: &Arc<Catalog>,
    pool: Option<Arc<Scheduler<SimRuntime>>>,
    sequences: Option<adtb::storage::Pid>,
) -> Database<SimRuntime> {
    let (pager, txns, heaps, indexes) = d.bindings();
    Database::new(
        rt.clone(),
        Setup {
            schema: schema.clone(),
            normal: normal.clone(),
            catalog: catalog.clone(),
            pager,
            txns,
            bindings: Bindings { heaps, indexes },
            budget: Budget::new(1 << 30),
            grant: 64 << 20,
            pool,
            options: exec::Options::default(),
            limits: Limits::default(),
            sequences,
        },
    )
    .expect("a database over the fixture")
}

/// Returns `v` with every value `map` names replaced.
fn rename(v: &Val, map: &BTreeMap<Val, Val>) -> Val {
    if let Some(w) = map.get(v) {
        return w.clone();
    }
    match v {
        Val::Ctor(c, fs) => Val::ctor(*c, fs.iter().map(|x| rename(x, map)).collect()),
        Val::Record(fs) => Val::record(fs.iter().map(|x| rename(x, map)).collect()),
        Val::Set(s) => Val::set(s.iter().map(|x| rename(x, map)).collect()),
        Val::List(xs) => Val::List(Arc::from(
            xs.iter().map(|x| rename(x, map)).collect::<Vec<_>>(),
        )),
        other => other.clone(),
    }
}

fn rename_rows(rows: &Rows, map: &BTreeMap<Val, Val>) -> Rows {
    rows.iter()
        .map(|(k, vs)| (*k, vs.iter().map(|v| rename(v, map)).collect()))
        .collect()
}

/// Adds to `out` the first field of every constructor value inside `v`: the keys of the rows
/// it holds.
fn firsts(v: &Val, out: &mut BTreeSet<Val>) {
    match v {
        Val::Ctor(_, fs) => {
            if let Some(k) = fs.first() {
                out.insert(k.clone());
            }
            fs.iter().for_each(|x| firsts(x, out));
        }
        Val::Record(fs) | Val::List(fs) => fs.iter().for_each(|x| firsts(x, out)),
        Val::Set(s) => s.iter().for_each(|x| firsts(x, out)),
        _ => {}
    }
}

/// Returns the keys of the rows `o` holds that `before` does not: the values a statement
/// generated.
fn new_keys(o: &Outcomes, before: &Rows) -> BTreeSet<Val> {
    let mut held = BTreeSet::new();
    before.values().flatten().for_each(|v| firsts(v, &mut held));
    let mut seen = BTreeSet::new();
    o.1.values().flatten().for_each(|v| firsts(v, &mut seen));
    for r in &o.0 {
        match r {
            Ok(g) => {
                firsts(&g.result, &mut seen);
                g.inserted
                    .values()
                    .chain(g.deleted.values())
                    .flatten()
                    .for_each(|v| firsts(v, &mut seen));
            }
            Err((_, report)) => report.iter().for_each(|v| firsts(v, &mut seen)),
        }
    }
    seen.difference(&held).cloned().collect()
}

fn rename_outcomes(o: &Outcomes, map: &BTreeMap<Val, Val>) -> Outcomes {
    let steps =
        o.0.iter()
            .map(|r| match r {
                Ok(g) => Ok(Got {
                    result: rename(&g.result, map),
                    inserted: rename_rows(&g.inserted, map),
                    deleted: rename_rows(&g.deleted, map),
                }),
                Err((what, report)) => Err((
                    what.clone(),
                    report.iter().map(|v| rename(v, map)).collect(),
                )),
            })
            .collect();
    (steps, rename_rows(&o.1, map))
}

fn permutations(xs: &[Val]) -> Vec<Vec<Val>> {
    if xs.len() <= 1 {
        return vec![xs.to_vec()];
    }
    let mut out = Vec::new();
    for i in 0..xs.len() {
        let mut rest = xs.to_vec();
        let x = rest.remove(i);
        for mut p in permutations(&rest) {
            p.insert(0, x.clone());
            out.push(p);
        }
    }
    out
}

/// Returns whether the engine's outcomes are the reference's up to a renaming of the keys the
/// engine generated.
fn agree(case: &Case, want: &Outcomes, got: &Outcomes) -> Result<(), String> {
    if want == got {
        return Ok(());
    }
    let f = (case.fixture)();
    let before = erase_db(&f.db);
    let mine: Vec<Val> = new_keys(got, &before).into_iter().collect();
    let theirs: Vec<Val> = new_keys(want, &before).into_iter().collect();
    if mine.len() == theirs.len() && mine.len() <= 6 && !mine.is_empty() {
        for p in permutations(&theirs) {
            let map: BTreeMap<Val, Val> = mine.iter().cloned().zip(p).collect();
            if rename_outcomes(got, &map) == *want {
                return Ok(());
            }
        }
    }
    let mut diff = String::new();
    for (i, (w, g)) in want.0.iter().zip(&got.0).enumerate() {
        if w != g {
            diff.push_str(&format!(
                "  statement {i} `{}`:\n    want {w:?}\n    got  {g:?}\n",
                case.steps[i].text
            ));
        }
    }
    if want.1 != got.1 {
        diff.push_str(&format!(
            "  final state:\n    want {:?}\n    got  {:?}\n",
            want.1, got.1
        ));
    }
    Err(format!("{}:\n{diff}", case.name))
}

fn key(i: i64) -> Value {
    Value::key(Value::Int(i))
}

fn text(t: &str) -> Value {
    Value::text(t)
}

fn identity_key(n: u8) -> Value {
    fixture().one(&format!("[{}]", uuid(n)))
}

const ON_KEY: &str = r#"insert identities.Google { id: k, user: u, gid: g }
unless conflict on key
else match old
  | Google o    -> update o set Google { o with gid: g }
  | EmailPass e -> fail "identity is email"
  | Apple a     -> fail "identity is apple""#;

const GET_OR_CREATE: &str = "transact
  let g = insert identities.Google { user: u, gid: x }
          unless conflict on (gid) in Google
          else update old set old
  return key g";

const INVOICES: &str = "type Status = Draft | Paid { ref: Text }
type Invoice = { id: Key invoices, status: Status, n: I64 }
relation invoices : Invoice key id generate sequence unique (status.Paid.ref)";

fn invoices() -> Fixture {
    Fixture::new(INVOICES)
}

fn deprecated_apple() -> Fixture {
    let mut f = fixture();
    let (_, apple) = f.ctor("Identity", "Apple");
    let identities = f.rel("identities");
    f.schema
        .stages
        .insert((identities, apple), adtb_lang::schema::Stage::Deprecated);
    f
}

fn declared_intl() -> Fixture {
    let mut f = fixture();
    let (address, intl) = f.ctor("Address", "Intl");
    f.schema
        .value_stages
        .insert((address, intl), adtb_lang::schema::Stage::Declared);
    f
}

/// The write examples of the reference interpreter's tests, as statements.
#[expect(
    clippy::too_many_lines,
    reason = "the write examples would have to be split by statement kind"
)]
fn cases() -> Vec<Case> {
    let u = |i: i64| ("u", "Key users", key(i));
    let ik = |n: u8| ("k", "Key identities", identity_key(n));
    vec![
        Case { name: "insert of an existing key", fixture, steps: vec![step(r#"insert users { id: users.key(1), name: "ann" }"#)] },
        Case { name: "generated key", fixture, steps: vec![with("insert users { name: n }", vec![("n", "Text", text("eve"))])] },
        Case { name: "insert into a constructor relation", fixture, steps: vec![with("insert identities.Google { user: u, gid: g }", vec![u(3), ("g", "Text", text("g-cy"))])] },
        Case { name: "insert from a query", fixture, steps: vec![
            step("delete i in identities.Google where i.gid == \"s-bo\""),
            step("insert identities.Google from (from x in imports.Oauth select { user: x.user, gid: x.sub })"),
        ] },
        Case { name: "insert into a sum", fixture, steps: vec![
            step("delete i in identities.Google where i.gid == \"s-bo\""),
            step("insert identities from (
                    from x in imports
                    select match x
                      | Email e -> EmailPass { user: e.user, email: e.addr, hash: e.h }
                      | Oauth o -> Google { user: o.user, gid: o.sub })"),
        ] },
        Case { name: "two rows with one key", fixture, steps: vec![step(r#"(insert users { id: users.key(9), name: "a" }, insert users { id: users.key(9), name: "b" })"#)] },
        Case { name: "two updates to different values", fixture, steps: vec![step(r#"(update u in users where key u == users.key(1) set User { u with name: "x" },
                update u in users where key u == users.key(1) set User { u with name: "y" })"#)] },
        Case { name: "two equal updates", fixture, steps: vec![step(r#"(update u in users where key u == users.key(1) set User { u with name: "x" },
                update u in users where key u == users.key(1) set User { u with name: "x" })"#)] },
        Case { name: "a delete and an insert of one key", fixture, steps: vec![step(r#"(delete u in users where key u == users.key(1),
                insert users { id: users.key(1), name: "new" })"#)] },
        Case { name: "an update that changes nothing", fixture, steps: vec![step("update i in identities where i.user == users.key(1)
             set match i
               | EmailPass e -> EmailPass { e with hash: rehash(e.hash) }
               | Google g    -> g
               | Apple a     -> a")] },
        Case { name: "one snapshot", fixture, steps: vec![step(r#"(insert users { name: "eve" }, update u in users set User { u with name: u.name ++ "!" })"#)] },
        Case { name: "a fault", fixture, steps: vec![with(r#"(insert users { name: "eve" }, if 1 / z > 0 then skip else skip)"#, vec![("z", "I64", Value::Int(0))])] },
        Case { name: "fail", fixture, steps: vec![
            step(r#"(insert users { name: "eve" }, fail "no")"#),
            step(r#"if false then fail "no" else insert users { name: "eve" }"#),
        ] },
        Case { name: "try does not catch fail", fixture, steps: vec![step(r#"transact let x = try(1) fail "stop" return x"#)] },
        Case { name: "a block binds generated keys", fixture, steps: vec![with("transact
               let u = insert users { name: n }
               let i = insert identities.Google { user: key u, gid: g }
               return key i", vec![("n", "Text", text("eve")), ("g", "Text", text("g-eve"))])] },
        Case { name: "a nested insert", fixture, steps: vec![step(r#"insert identities.Google { user: key (insert users { name: "nest" }), gid: "g-n" }"#)] },
        Case { name: "a sequence never reissues", fixture, steps: vec![
            step(r#"insert users { id: users.key(42), name: "imported" }"#),
            step("delete u in users where key u == users.key(42)"),
            step(r#"insert users { name: "next" }"#),
        ] },
        Case { name: "generated keys are distinct", fixture, steps: vec![step(r#"insert users from (from u in users select User { name: u.name ++ "2" })"#)] },
        Case { name: "one clock value", fixture, steps: vec![at(777, "update s in logins where s.user == users.key(1) set Revoked { }")] },
        Case { name: "rejects into", fixture, steps: vec![step(r#"insert users from (
                 from r in raws select User { name: if r.n > 0 then "ok" else reject "negative" })
               rejects into user_rejects"#)] },
        Case { name: "bad input", fixture, steps: vec![step(r#"insert users from (
                 from r in raws select User { name: if r.n > 0 then "ok" else reject "negative" })"#)] },
        Case { name: "an update of a constructor relation", fixture, steps: vec![with("update g in identities.Google where g.user == u set Google { g with gid: g2 }", vec![u(2), ("g2", "Text", text("g2"))])] },
        Case { name: "an update over a key-preserving query", fixture, steps: vec![with("update g in (from g in identities.Google, x in imports.Oauth
                     where g.gid == x.sub select g)
         set Google { g with user: u }", vec![u(2)])] },
        Case { name: "an update rebuilds a value-level sum", fixture, steps: vec![with(r#"update c in customers where c.id == k
           set Customer { c with address: match c.address
                                            | US a   -> US { a with zip: "02139" }
                                            | Intl i -> i }"#, vec![("k", "Key customers", key(1))])] },
        Case { name: "key changed", fixture, steps: vec![with("update u in users where key u == users.key(1) set v", vec![("v", "User", Value::Ctor(0, vec![("id".into(), key(2)), ("name".into(), text("x"))]))])] },
        Case { name: "a retag", fixture, steps: vec![step(&format!("update i in identities where i.id == {}
             set match i
               | EmailPass e -> Google {{ gid: lookup(e.email) }}
               | Google g    -> g
               | Apple a     -> a", uuid(5)))] },
        Case { name: "a retag fills omitted fields", fixture, steps: vec![at(99, "update s in logins where s.user == users.key(1) set Revoked { }")] },
        Case { name: "a retag refused by a reference", fixture, steps: vec![step(&format!(r#"update i in identities.EmailPass where i.id == {} set Google {{ gid: "g" }}"#, uuid(1)))] },
        Case { name: "a retag with its referrers deleted", fixture, steps: vec![step(&format!(r#"transact
             delete r in resets where r.identity == {k}
             update i in identities.EmailPass where i.id == {k} set Google {{ gid: "g" }}"#, k = uuid(1)))] },
        Case { name: "a let on an update", fixture, steps: vec![step(r#"transact
                 let x = update u in users where key u == users.key(1) set User { u with name: "z" }
                 return from y in x select y.name"#)] },
        Case { name: "a delete over a constructor relation", fixture, steps: vec![with("delete s in logins.Revoked where s.user == u", vec![u(1)])] },
        Case { name: "a delete over a key-preserving query", fixture, steps: vec![with("delete s in (from s in logins.Active, i in identities.Google
                      where s.identity == key i and i.gid == g select s)", vec![("g", "Text", text("g-bo"))])] },
        Case { name: "a let on a delete", fixture, steps: vec![step(r#"transact
                 let x = delete n in notes where n.body == "plain"
                 return from y in x select y.body"#)] },
        Case { name: "cascade and retag", fixture, steps: vec![at(1234, &format!("delete i in identities where i.id == {}", uuid(1)))] },
        Case { name: "restrict", fixture, steps: vec![step("delete x in users where key x == users.key(2)")] },
        Case { name: "referrers deleted with the target", fixture, steps: vec![with("transact
           delete d in badges where d.owner == u
           delete x in users where key x == u", vec![u(4)])] },
        Case { name: "only the target of a restrict", fixture, steps: vec![step("delete x in users where key x == users.key(4)")] },
        Case { name: "a cascaded row", fixture, steps: vec![step("transact
            delete i in identities where i.user == users.key(1)
            delete t in tickets
            delete x in users where key x == users.key(1)")] },
        Case { name: "a repointed reference", fixture, steps: vec![step(&format!("transact
           update s in logins.Active where s.id == logins.key(1) set Active {{ s with identity: {} }}
           delete i in identities where i.id == {}", uuid(2), uuid(1)))] },
        Case { name: "an inserted row referencing a gone key", fixture, steps: vec![step(r#"transact
             insert notes { author: Some { value: users.key(4) }, body: "new" }
             delete d in badges where d.owner == users.key(4)
             delete t in tickets
             delete x in users where key x == users.key(4)"#)] },
        Case { name: "a retag and a clear", fixture, steps: vec![step(&format!("transact
           delete d in badges where d.owner == users.key(4)
           delete i in identities where i.id == {}
           delete x in users where key x == users.key(4)", uuid(2)))] },
        Case { name: "a retag of the target", fixture, steps: vec![step(&format!(r#"update i in identities.Google where i.id == {} set Apple {{ sub: "a" }}"#, uuid(3)))] },
        Case { name: "a retag, by parameter", fixture, steps: vec![with("update i in identities where i.id == k
             set match i
               | EmailPass e -> Google { gid: e.email }
               | Google g    -> g
               | Apple a     -> a", vec![ik(5)])] },
        Case { name: "a retag refused by a reference, by parameter", fixture, steps: vec![with(r#"update i in identities.EmailPass where i.id == k set Google { gid: "g" }"#, vec![ik(1)])] },
        Case { name: "a retag with its referrers deleted, by parameter", fixture, steps: vec![with(r#"transact
             delete r in resets where r.identity == k
             update i in identities.EmailPass where i.id == k set Google { gid: "g" }"#, vec![ik(1)])] },
        Case { name: "cascade and retag, by parameter", fixture, steps: vec![Step { clock: 1234, ..with("delete i in identities where i.id == k", vec![ik(1)]) }] },
        Case { name: "a repointed reference, by parameter", fixture, steps: vec![with("transact
           update s in logins.Active where s.id == logins.key(1) set Active { s with identity: j }
           delete i in identities where i.id == k", vec![("j", "Key identities", identity_key(2)), ik(1)])] },
        Case { name: "a retag and a clear, by parameter", fixture, steps: vec![with("transact
           delete d in badges where d.owner == users.key(4)
           delete i in identities where i.id == k
           delete x in users where key x == users.key(4)", vec![ik(2)])] },
        Case { name: "a retag of the target, by parameter", fixture, steps: vec![with(r#"update i in identities.Google where i.id == k set Apple { sub: "a" }"#, vec![ik(3)])] },
        Case { name: "an upsert on a unique", fixture, steps: vec![with("insert identities.Google { user: u, gid: g }
         unless conflict on (gid) in Google
         else update old set Google { old with user: u }", vec![u(2), ("g", "Text", text("g-ann"))])] },
        Case { name: "an upsert with no conflict", fixture, steps: vec![with("insert identities.Google { user: u, gid: g }
         unless conflict on (gid) in Google
         else update old set Google { old with user: u }", vec![u(2), ("g", "Text", text("fresh"))])] },
        Case { name: "an upsert on the key", fixture, steps: vec![
            with(ON_KEY, vec![("k", "Key identities", identity_key(2)), u(1), ("g", "Text", text("g-new"))]),
            with(ON_KEY, vec![("k", "Key identities", identity_key(1)), u(1), ("g", "Text", text("g-x"))]),
        ] },
        Case { name: "an upsert on a unique over expressions", fixture, steps: vec![
            with("insert identities.EmailPass { user: u, email: e, hash: h }
         unless conflict on (lower(email)) in EmailPass
         else update old set old", vec![u(3), ("e", "Text", text("ANN@example.com")), ("h", "Bytes", Value::Bytes(b"h".to_vec()))]),
            step(r#"insert identities.EmailPass { user: users.key(3), email: "ANN@example.com", hash: bytes("h") }"#),
        ] },
        Case { name: "a bulk upsert", fixture, steps: vec![step("insert identities.Google from (
            from x in imports.Oauth select { user: x.user, gid: x.sub })
          unless conflict on (gid) in Google
          else update old set Google { old with user: new.user }")] },
        Case { name: "get or create, existing", fixture, steps: vec![with(GET_OR_CREATE, vec![u(2), ("x", "Text", text("g-ann"))])] },
        Case { name: "get or create, created", fixture, steps: vec![with(GET_OR_CREATE, vec![u(2), ("x", "Text", text("g-new"))])] },
        Case { name: "an upsert binding", fixture, steps: vec![with(r#"transact
                 let b = insert identities.Google { id: k, user: users.key(2), gid: "gg" }
                         unless conflict on key
                         else match old
                           | Google o    -> update o set o
                           | EmailPass e -> update e set Google { gid: "from-email" }
                           | Apple a     -> update a set a
                 return match b | Google g -> g.gid | Apple a -> a.sub"#, vec![("k", "Key identities", identity_key(5))])] },
        Case { name: "two proposed rows with one unique value", fixture, steps: vec![step(r#"insert identities.Google from (
             from p in [{ u: users.key(1), g: "dup" }, { u: users.key(2), g: "dup" }]
             select { user: p.u, gid: p.g })
           unless conflict on (gid) in Google
           else update old set old"#)] },
        Case { name: "an upsert does not see another insert", fixture, steps: vec![step(r#"(insert identities.Google { user: users.key(1), gid: "z" },
            insert identities.Google { user: users.key(2), gid: "z" }
            unless conflict on (gid) in Google
            else update old set old)"#)] },
        Case { name: "an upsert on the key meets another insert", fixture, steps: vec![step(r#"(insert users { id: users.key(9), name: "a" },
                insert users { id: users.key(9), name: "b" }
                unless conflict on key
                else update old set old)"#)] },
        Case { name: "a constraint violation", fixture, steps: vec![step(r#"insert identities.Google { user: users.key(3), gid: "" }"#)] },
        Case { name: "the least name", fixture, steps: vec![
            step(r#"(insert identities.Google { user: users.key(3), gid: "" },
            insert identities.Google { user: users.key(4), gid: "" })"#),
            step(r#"insert users { id: users.key(1), name: "aa" }"#),
            step(r#"insert users { id: users.key(1), name: "zz" }"#),
        ] },
        Case { name: "a row check", fixture, steps: vec![step(r#"insert users { name: "a name longer than twenty" }"#)] },
        Case { name: "a transition constraint", fixture, steps: vec![
            step(r#"delete n in notes where n.body == "frozen""#),
            step(r#"delete n in notes where n.body == "plain""#),
        ] },
        Case { name: "a domain", fixture, steps: vec![
            step(r#"insert contacts { email: Email("ab") }"#),
            step(r#"insert contacts { email: Email("abcd") }"#),
        ] },
        Case { name: "a fault in a constraint", fixture, steps: vec![step(r#"(insert raws { n: 0 }, insert users { id: users.key(1), name: "ann" })"#)] },
        Case { name: "a deprecated constructor relation", fixture: deprecated_apple, steps: vec![step(r#"insert identities.Apple { user: users.key(4), sub: "a-dee" }"#)] },
        Case { name: "a declared value-level constructor", fixture: declared_intl, steps: vec![
            step(r#"update c in customers where c.id == customers.key(1)
               set Customer { c with address: Intl { country: "de", postcode: "1" } }"#),
            step(r#"update c in customers where c.id == customers.key(2) set Customer { c with name: "e" }"#),
        ] },
        Case { name: "a block returns unit", fixture, steps: vec![step(r#"transact insert users { name: "eve" }"#)] },
        Case { name: "a block result reads the snapshot", fixture, steps: vec![step(r#"transact insert users { name: "eve" } return count(u for u in users)"#)] },
        Case { name: "a block with options", fixture, steps: vec![
            step("transact retry 3 read_only return count(u for u in users)"),
            step(r#"transact durability quorum insert users { name: "q" }"#),
        ] },
        Case { name: "a query statement", fixture, steps: vec![step("count(u for u in users)")] },
        Case { name: "a value-level unique", fixture: invoices, steps: vec![
            step("(insert invoices { status: Draft, n: 0 }, insert invoices { status: Draft, n: 1 })"),
            step(r#"insert invoices { status: Paid { ref: "r1" }, n: 2 }"#),
            step(r#"insert invoices { status: Paid { ref: "r1" }, n: 3 }"#),
        ] },
        Case { name: "an upsert on a value-level path", fixture: invoices, steps: vec![
            step(r#"insert invoices { status: Paid { ref: "r1" }, n: 1 }"#),
            step(r#"insert invoices { status: Paid { ref: "r1" }, n: 2 }
               unless conflict on (status.Paid.ref)
               else update old set Invoice { old with n: old.n + 10 }"#),
            step(r"insert invoices { status: Draft, n: 5 }
            unless conflict on (status.Paid.ref)
            else update old set Invoice { old with n: 0 }"),
        ] },
    ]
}

fn check_all(seeds: Range<u64>, mode: Mode) {
    let mut bad = Vec::new();
    let mut runs = 0;
    for case in &cases() {
        let want = reference(case);
        for seed in support::sweep(seeds.clone()) {
            runs += 1;
            match panic::catch_unwind(AssertUnwindSafe(|| engine(seed, case, mode))) {
                Ok(got) => {
                    if let Err(d) = agree(case, &want, &got) {
                        bad.push(format!("seed {seed}: {d}"));
                    }
                }
                Err(payload) => bad.push(format!(
                    "seed {seed}: {}: panicked: {}",
                    case.name,
                    panic_text(&*payload)
                )),
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {runs} runs differ from the reference:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

fn panic_text(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("a panic without a message")
}

#[test]
fn every_write_example_agrees_with_the_reference() {
    check_all(
        1..2,
        Mode {
            workers: 0,
            one: false,
        },
    );
}

#[test]
fn write_examples_in_one_transaction_agree_and_failed_statements_leave_no_effect() {
    check_all(
        1..2,
        Mode {
            workers: 0,
            one: true,
        },
    );
}

#[test]
fn write_examples_on_several_workers_agree_for_every_seed() {
    check_all(
        0..3,
        Mode {
            workers: 3,
            one: false,
        },
    );
}

const SHOP: &str = "type Counter = { id: Key counters, n: I64 }
relation counters : Counter key id generate sequence
type Label = { id: Key tags, name: Text }
relation tags : Label key id generate sequence unique (name)";

fn shop() -> Fixture {
    let mut f = Fixture::new(SHOP);
    f.rows("counters", "[Counter { id: counters.key(1), n: 0 }]");
    f
}

const INC: &str =
    "update c in counters where key c == counters.key(1) set Counter { c with n: c.n + 1 }";
const TAG: &str = "transact let x = insert tags { name: t } return key x";
const UPSERT: &str = "insert tags { name: t } unless conflict on (name) else update old set old";
const DROP: &str = "delete g in tags where g.name == t";
const FAILING: &str =
    "(update c in counters where key c == counters.key(1) set Counter { c with n: c.n + 1 },
  insert tags { name: \"twice\" },
  insert tags { name: \"twice\" })";

/// A prepared statement over parameter `t`, if it has one.
struct Prep {
    p: Prepared,
    t: Option<Var>,
}

/// Prepare `text`, a statement over parameter `t` of type `Text` when `over_t`.
fn prep(f: &Fixture, text: &str, over_t: bool) -> Prep {
    let params = if over_t {
        vec![("t", "Text", Value::text(""))]
    } else {
        Vec::new()
    };
    let p = prepare(f, &with(text, params));
    let t = p.params.first().map(|(v, _)| *v);
    Prep { p, t }
}

/// One finished transaction of a client: what it ran and its value when it committed, or how
/// it failed.
#[derive(Clone, Debug)]
struct Done {
    op: usize,
    result: Result<Val, String>,
}

fn run_txn(
    db: &Database<SimRuntime>,
    schema: &Schema,
    s: &Prep,
    name: &str,
) -> Result<Val, String> {
    run_txn_with(db, schema, s, name, None, &mut |_| {})
}

/// Run `s` over `name` in a transaction, then `failing`, a statement that must fail and leave
/// the transaction open, then commit. `seen` gets `s`'s result before the commit.
fn run_txn_with(
    db: &Database<SimRuntime>,
    schema: &Schema,
    s: &Prep,
    name: &str,
    failing: Option<&Prep>,
    seen: &mut dyn FnMut(&Val),
) -> Result<Val, String> {
    let fail = |tx, e: &write::Error| {
        db.abort(tx).expect("an abort");
        Err(format!("{:?}", engine_error(schema, e)))
    };
    let tx = db.begin().map_err(|e| format!("{e:?}"))?;
    let params: Vec<(Var, Val)> = s.t.map(|v| (v, Val::text(name))).into_iter().collect();
    let result = match db.statement(&tx, &s.p.terms, s.p.item, &params, 0) {
        Ok(o) => o.result,
        Err(e) => return fail(tx, &e),
    };
    seen(&result);
    if let Some(f) = failing {
        match db.statement(&tx, &f.p.terms, f.p.item, &[], 0) {
            Ok(_) => panic!("a failing statement succeeded"),
            Err(e) if e.ends_transaction() => return fail(tx, &e),
            Err(e) => {
                let e = format!("{:?}", engine_error(schema, &e));
                assert!(
                    e.contains("Violated app::tags.unique(name) in Label"),
                    "the failing statement failed by {e}"
                );
            }
        }
    }
    match db.commit(tx) {
        Ok(_) => Ok(result),
        Err(e) => Err(format!("{:?}", engine_error(schema, &e))),
    }
}

fn is_expected_failure(e: &str) -> bool {
    e.contains("Serialization") || e.contains("Violated app::tags.unique(name) in Label")
}

#[test]
fn concurrent_writers_keep_keys_and_uniques_and_lose_no_update() {
    const SEEDS: Range<u64> = 0..4;
    let f = shop();
    let catalog = Arc::new(planfix::catalog(&f.schema));
    let normal = Arc::new(norm::program(&f.schema, &f.program).expect("the schema normalises"));
    let schema = Arc::new(f.schema.clone());
    let preps = Arc::new([
        prep(&f, INC, false),
        prep(&f, TAG, true),
        prep(&f, UPSERT, true),
        prep(&f, DROP, true),
    ]);
    let failing = Arc::new(prep(&f, FAILING, false));
    let mut commits = 0;
    for seed in support::sweep(SEEDS) {
        let mut sim = Sim::new(seed, Config::default());
        let done: Arc<std::sync::Mutex<Vec<Done>>> = Arc::default();
        let mut finals = None;
        let result = sim.run(|rt| {
            use adtb::runtime::{Entropy, Threads};
            let world = World {
                schema: &f.schema,
                normal: &normal,
                catalog: &catalog,
                db: &f.db,
            };
            let d = Db::load_with(&rt, &world, false);
            let db = Arc::new(database(&rt, d, &schema, &normal, &catalog, None, None));
            let clients: Vec<_> = (0..4)
                .map(|c| {
                    let (db, rt2, preps, done, schema, failing) = (
                        db.clone(),
                        rt.clone(),
                        preps.clone(),
                        done.clone(),
                        schema.clone(),
                        failing.clone(),
                    );
                    rt.spawn(&format!("client {c}"), move || {
                        for _ in 0..6 {
                            let op = (rt2.next_u64() % 4) as usize;
                            let name = ["a", "b", "c"][(rt2.next_u64() % 3) as usize].to_string();
                            let also = (rt2.next_u64() % 2 == 0).then_some(failing.as_ref());
                            let result =
                                run_txn_with(&db, &schema, &preps[op], &name, also, &mut |_| {});
                            done.lock().unwrap().push(Done { op, result });
                        }
                    })
                })
                .collect();
            for c in clients {
                rt.join(c);
            }
            let tx = db.begin().expect("a transaction");
            finals = Some(db.rows(&tx).expect("the rows"));
            db.abort(tx).expect("an abort");
        });
        assert_eq!(Outcome::Finished, result, "seed {seed}");
        let rows = finals.expect("the run finished");
        let done = done.lock().unwrap().clone();
        for d in &done {
            if let Err(e) = &d.result {
                assert!(is_expected_failure(e), "seed {seed}: {d:?}");
            }
        }
        let counters = f.rel("counters");
        let incs = done
            .iter()
            .filter(|d| d.op == 0 && d.result.is_ok())
            .count() as i64;
        let n: Vec<Val> = rows[&(counters, 0)].iter().map(|r| field(r, 1)).collect();
        assert_eq!(
            vec![Val::Int(incs)],
            n,
            "seed {seed}: every committed increment counts"
        );
        let tags = f.rel("tags");
        let names: Vec<Val> = rows
            .get(&(tags, 0))
            .into_iter()
            .flatten()
            .map(|r| field(r, 1))
            .collect();
        let distinct: BTreeSet<&Val> = names.iter().collect();
        assert_eq!(
            distinct.len(),
            names.len(),
            "seed {seed}: tag names are unique: {names:?}"
        );
        let keys: Vec<&Val> = done
            .iter()
            .filter(|d| d.op == 1)
            .filter_map(|d| d.result.as_ref().ok())
            .collect();
        let distinct_keys: BTreeSet<&&Val> = keys.iter().collect();
        assert_eq!(
            distinct_keys.len(),
            keys.len(),
            "seed {seed}: generated keys are distinct"
        );
        commits += done.iter().filter(|d| d.result.is_ok()).count();
    }
    assert!(
        commits > share(SEEDS, 20),
        "only {commits} transactions committed"
    );
}

#[test]
fn committed_writes_survive_a_crash_and_sequences_never_reissue() {
    use adtb::runtime::Threads;
    let f = Fixture::new(SHOP);
    let catalog = Arc::new(planfix::catalog(&f.schema));
    let normal = Arc::new(norm::program(&f.schema, &f.program).expect("the schema normalises"));
    let schema = Arc::new(f.schema.clone());
    let tag = Arc::new(prep(&f, TAG, true));
    let world = World {
        schema: &f.schema,
        normal: &normal,
        catalog: &catalog,
        db: &f.db,
    };
    type Slot<T> = Arc<std::sync::Mutex<T>>;
    let load = |rt: &SimRuntime,
                roots: &Slot<Option<support::Roots>>,
                acked: &Slot<BTreeMap<String, Val>>,
                submitted: &Slot<BTreeSet<String>>,
                drawn: &Slot<BTreeSet<Val>>,
                started: &Slot<Option<u64>>| {
        let (d, mut r) = Db::build(rt, &world, None).expect("a new database");
        let db = Arc::new(database(rt, d, &schema, &normal, &catalog, None, None));
        r.sequences = Some(db.sequences_root());
        *roots.lock().unwrap() = Some(r);
        started.lock().unwrap().replace({
            use adtb::runtime::Clock;
            rt.now().as_nanos()
        });
        let clients: Vec<_> = (0..3)
            .map(|c| {
                let (db, tag, acked, submitted, schema, drawn) = (
                    db.clone(),
                    tag.clone(),
                    acked.clone(),
                    submitted.clone(),
                    schema.clone(),
                    drawn.clone(),
                );
                rt.spawn(&format!("client {c}"), move || {
                    for i in 0..40 {
                        let name = format!("c{c}-{i}");
                        submitted.lock().unwrap().insert(name.clone());
                        let mut seen = |k: &Val| {
                            drawn.lock().unwrap().insert(k.clone());
                        };
                        match run_txn_with(&db, &schema, &tag, &name, None, &mut seen) {
                            Ok(k) => {
                                acked.lock().unwrap().insert(name, k);
                            }
                            Err(_) => {
                                submitted.lock().unwrap().remove(&name);
                            }
                        }
                    }
                })
            })
            .collect();
        for c in clients {
            rt.join(c);
        }
    };
    let (mut crashed, mut lost_draws) = (0, 0);
    for seed in 0..4 {
        let mut dry = Sim::new(seed, Config::default());
        let started: Slot<Option<u64>> = Arc::default();
        dry.run(|rt| {
            load(
                &rt,
                &Arc::default(),
                &Arc::default(),
                &Arc::default(),
                &Arc::default(),
                &started,
            );
        });
        let begun = started.lock().unwrap().expect("the clients started");
        let span = dry.now().as_nanos() - begun;
        let mut sim = Sim::new(seed, Config::default());
        let acked: Arc<std::sync::Mutex<BTreeMap<String, Val>>> = Arc::default();
        let submitted: Arc<std::sync::Mutex<BTreeSet<String>>> = Arc::default();
        let roots: Arc<std::sync::Mutex<Option<support::Roots>>> = Arc::default();
        let drawn: Slot<BTreeSet<Val>> = Arc::default();
        let at = begun + span * (1 + seed) / 6;
        sim.crash_at(adtb::runtime::Instant::from_nanos(at));
        let first = sim.run(|rt| load(&rt, &roots, &acked, &submitted, &drawn, &Arc::default()));
        if first == Outcome::Crashed && !acked.lock().unwrap().is_empty() {
            crashed += 1;
        }
        let roots = roots.lock().unwrap().clone();
        let Some(roots) = roots else {
            assert!(
                acked.lock().unwrap().is_empty(),
                "seed {seed}: commits were acknowledged before the roots were recorded"
            );
            continue;
        };
        let mut after = None;
        let result = sim.run(|rt| {
            let (d, _) = Db::build(&rt, &world, Some(&roots))
                .unwrap_or_else(|| panic!("seed {seed}: the database does not reopen"));
            let db = database(&rt, d, &schema, &normal, &catalog, None, roots.sequences);
            let tx = db.begin().expect("a transaction");
            let rows = db.rows(&tx).expect("the rows");
            db.abort(tx).expect("an abort");
            let next = run_txn(&db, &schema, &tag, "after").expect("an insert after recovery");
            after = Some((rows, next));
        });
        assert_eq!(Outcome::Finished, result, "seed {seed}");
        let (rows, next) = after.unwrap_or_else(|| panic!("seed {seed}: recovery read no rows"));
        let held: BTreeMap<String, Val> = rows
            .get(&(f.rel("tags"), 0))
            .into_iter()
            .flatten()
            .map(|r| {
                let Val::Text(n) = field(r, 1) else {
                    panic!("a name")
                };
                (n.to_string(), field(r, 0))
            })
            .collect();
        for (name, k) in acked.lock().unwrap().iter() {
            assert_eq!(
                Some(k),
                held.get(name),
                "seed {seed}: acknowledged {name} survives"
            );
        }
        let submitted = submitted.lock().unwrap();
        for name in held.keys() {
            assert!(
                submitted.contains(name),
                "seed {seed}: {name} was never submitted"
            );
        }
        assert!(
            !held.values().any(|k| *k == next),
            "seed {seed}: the key {next:?} drawn after recovery was held"
        );
        let drawn = drawn.lock().unwrap();
        let in_flight = submitted.len() > acked.lock().unwrap().len();
        lost_draws +=
            usize::from(in_flight && drawn.iter().any(|k| !held.values().any(|h| h == k)));
        let last = drawn.last().cloned();
        assert!(
            last.is_none_or(|d| next > d),
            "seed {seed}: the key {next:?} drawn after recovery was drawn before the crash"
        );
    }
    eprintln!("{crashed} runs crashed after a commit, {lost_draws} lost a key drawn in flight");
    assert!(crashed > 0, "no run crashed after acknowledging a commit");
    assert!(lost_draws > 0, "no crash lost a key drawn in flight");
}

/// Begin a transaction, have a second one run `s` and commit, then damage the commit log page
/// holding the second's status on the device, evict it, and run `s` in the first. Returns the
/// damaged page and how the first's statement ended.
fn behind_a_damaged_commit_log(
    seed: u64,
    text: &str,
    over_t: bool,
) -> (adtb::storage::Pid, Result<(), String>) {
    use adtb::runtime::Runtime;
    use adtb::storage::Pid;
    let f = shop();
    let catalog = Arc::new(planfix::catalog(&f.schema));
    let normal = Arc::new(norm::program(&f.schema, &f.program).expect("the schema normalises"));
    let schema = Arc::new(f.schema.clone());
    let s = prep(&f, text, over_t);
    let mut out = None;
    let mut sim = Sim::new(seed, Config::default());
    let result = sim.run(|rt| {
        let world = World {
            schema: &f.schema,
            normal: &normal,
            catalog: &catalog,
            db: &f.db,
        };
        let d = Db::load_with(&rt, &world, false);
        let status = d.txns.commit_log().status_pages()[&0];
        let db = database(&rt, d, &schema, &normal, &catalog, None, None);
        let params: Vec<(Var, Val)> = s.t.map(|v| (v, Val::text("x"))).into_iter().collect();
        let first = db.begin().expect("a transaction");
        let second = db.begin().expect("a transaction");
        db.statement(&second, &s.p.terms, s.p.item, &params, 0)
            .expect("the second statement");
        db.commit(second).expect("the second commits");
        let pager = db.pager().clone();
        pager.flush_log();
        pager.flush_pages(usize::MAX);
        for p in 0..4096 {
            pager.read(Pid(1 << 16 | p)).expect("a page never written");
        }
        let mut io = rt.io();
        let (file, _) = common::open(&rt, &mut io, "app.adt", false).expect("the data file");
        let at = status.0 * support::harness::PAGE as u64;
        common::write(&rt, &mut io, file, at, &[0xa5; support::harness::PAGE]).expect("a write");
        common::sync(&rt, &mut io, file).expect("a sync");
        let ended = db.statement(&first, &s.p.terms, s.p.item, &params, 0);
        out = Some((status, ended.map(|_| ()).map_err(|e| format!("{e:?}"))));
    });
    assert_eq!(Outcome::Finished, result, "seed {seed}");
    out.expect("the run finished")
}

#[test]
fn a_write_meeting_a_damaged_commit_log_fails_the_statement() {
    for seed in 0..2 {
        let (status, ended) = behind_a_damaged_commit_log(seed, INC, false);
        let e = ended.expect_err("the status does not read");
        let want = format!("Page(PageError {{ pid: {status:?}");
        assert!(
            e.starts_with(&want),
            "seed {seed}: the update failed by {e}"
        );
    }
}

#[test]
fn a_unique_probe_meeting_a_damaged_commit_log_fails_the_statement() {
    for seed in 0..2 {
        let (status, ended) = behind_a_damaged_commit_log(seed, "insert tags { name: t }", true);
        let e = ended.expect_err("the status does not read");
        let want = format!("Page(PageError {{ pid: {status:?}");
        assert!(
            e.starts_with(&want),
            "seed {seed}: the insert failed by {e}"
        );
    }
}

#[test]
fn only_a_sequence_key_has_a_counter() {
    use adtb::storage::HEADER_LEN;
    let f = Fixture::new(
        "type Item = { id: I64, n: I64 }
relation items : Item key id
type Label = { id: Key tags, name: Text }
relation tags : Label key id generate sequence",
    );
    let catalog = Arc::new(planfix::catalog(&f.schema));
    let normal = Arc::new(norm::program(&f.schema, &f.program).expect("the schema normalises"));
    let schema = Arc::new(f.schema.clone());
    let item = prep(&f, "insert items { id: 7, n: 0 }", false);
    let tag = prep(&f, "insert tags { name: \"a\" }", false);
    let mut counters = None;
    let mut sim = Sim::new(0, Config::default());
    finished(&mut sim, |rt| {
        let world = World {
            schema: &f.schema,
            normal: &normal,
            catalog: &catalog,
            db: &f.db,
        };
        let d = Db::load_with(&rt, &world, false);
        let db = database(&rt, d, &schema, &normal, &catalog, None, None);
        run_txn(&db, &schema, &item, "").expect("the item commits");
        run_txn(&db, &schema, &tag, "").expect("the tag commits");
        let page = db.pager().read(db.sequences_root()).expect("the counters");
        let n = &page[HEADER_LEN..HEADER_LEN + 4];
        counters = Some(u32::from_le_bytes(n.try_into().expect("4 bytes")));
    });
    assert_eq!(Some(1), counters, "only tags has a counter");
}

fn field(r: &Val, i: usize) -> Val {
    r.fields().expect("a row has fields")[i].clone()
}
