use crate::plan::fixture;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Bound, ControlFlow};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::batch;
use crate::exec::{
    self, Broken, Budget, Call, Error, Exceeded, Options as XOptions, ProbeBounds, Program, Reads,
    Routes, Rows, Scheduler, StopsEarly, Store, Val,
};
use crate::heap::{FieldType, HeapNo};
use crate::key::{CtorId, Tid};
use crate::mvcc::{ScanId, Scope, SerializationFailure, Writers};
use crate::plan::{Error as PlanError, HeapId, IndexId, Options as PlanOptions, Rewrites, plan};
use crate::runtime::{IoError, OpKind};
use crate::storage::Pid;
use crate::testing::Knobs;
use crate::{OpenOptions, TxnCallOptions, TxnOptions};
use adtb_lang::kernel::Var;
use adtb_lang::schema::{Committed, Schema};
use adtb_lang::types::Ty;
use adtb_reference::{Abort, R, Value};
use adtb_sim::{Action, Config, Match, Plan, Sim, SimRuntime};
use adtb_testkit::uuid;

use fixture::{Fixture, variants};

use crate::catalog::layout::field_type;
use crate::test_world::finished;
use crate::test_world::load;
use crate::test_world::support::{row_ty, to_ref, to_val};
use adtb_testkit::seeds::sweep;

/// Returns `f` as [`planner`] answers it over a new database of `f`'s declarations, opening one
/// database per declarations.
fn planned(f: &Fixture) -> Fixture {
    thread_local! {
        static PLANNED: RefCell<BTreeMap<String, (Committed, crate::plan::Catalog)>> =
            const { RefCell::new(BTreeMap::new()) };
    }
    let known = PLANNED.with_borrow(|p| p.get(f.decls()).cloned());
    let (schema, catalog) = known.unwrap_or_else(|| {
        let mut out = None;
        let mut sim = Sim::new(0, Config::default());
        finished(&mut sim, |rt| {
            let db = load::open(&rt, f.decls());
            out = Some((db.schema().clone(), db.testing().plan_catalog()));
        });
        let got = out.expect("the run finished");
        PLANNED.with_borrow_mut(|p| p.insert(f.decls().to_owned(), got.clone()));
        got
    });
    let rows = adtb_testkit::load::rows_of(&schema, &f.schema, &f.db);
    f.rebound(schema, catalog, rows)
}

/// Returns a simulator of `seed` holding the database of `f`'s declarations with its rows loaded
/// as [`load::committed_noise`] loads them, made once per declarations and rows.
fn loaded(seed: u64, f: &Fixture) -> Sim {
    let files = load::files(f.decls(), &f.schema, &f.db, &options());
    Sim::with_files(seed, Config::default(), files)
}

/// Returns the options of a database whose versions only the test's own writes change: no
/// background vacuum.
pub(crate) fn options() -> OpenOptions {
    OpenOptions {
        testing: Knobs::without_background_vacuum(),
        ..load::options(1)
    }
}

pub(crate) fn read_only() -> TxnOptions {
    TxnOptions {
        read_only: true,
        ..TxnOptions::default()
    }
}

fn running() -> Fixture {
    planned(&fixture::running())
}

/// One plan to run, with what the reference interpreter says it computes.
struct Case {
    name: String,
    rewrites: Rewrites,
    program: Arc<Program>,
    params: Vec<(Var, Val)>,
    ty: Ty,
    want: R<Value>,
}

impl Case {
    fn bound(&self) -> exec::Bound {
        self.program
            .clone()
            .bind(|v| {
                self.params
                    .iter()
                    .find(|(p, _)| *p == v)
                    .map(|(_, x)| x.clone())
            })
            .unwrap_or_else(|e| panic!("{} binds its parameters: {e:?}", self.name))
    }
}

/// Returns a case per rewrite variant of ad hoc `text` over `params`, each given as name, type
/// and value.
fn cases_of(f: &Fixture, text: &str, params: &[(&str, &str, Value)]) -> Vec<Case> {
    let types: Vec<(&str, &str)> = params.iter().map(|(n, t, _)| (*n, *t)).collect();
    let (n, t) = f.normal(text, &types);
    let bound: Vec<(Var, Value)> = params
        .iter()
        .map(|(name, _, v)| (n.params()[*name], v.clone()))
        .collect();
    let want = f.reference(text, params);
    let ty = n.terms().ty(t).clone();
    variants()
        .into_iter()
        .map(|(name, rewrites)| {
            let p = plan(
                &f.schema,
                &f.normal,
                &f.catalog,
                n.valid(),
                t,
                PlanOptions {
                    rewrites,
                    hiding: false,
                },
            )
            .unwrap_or_else(|e| panic!("`{text}` has no plan: {e}"));
            let program = exec::compile(&f.schema, &f.catalog, &p, None)
                .unwrap_or_else(|e| panic!("`{text}` does not compile: {e:?}"));
            let params = bound
                .iter()
                .map(|(v, x)| (*v, to_val(&f.schema, &p.terms.var_info(*v).ty, x)))
                .collect();
            Case {
                name: format!("`{text}` with {name}"),
                rewrites,
                program: Arc::new(program),
                params,
                ty: ty.clone(),
                want: want.clone(),
            }
        })
        .collect()
}

/// Returns the reference outcome an executor outcome stands for.
fn outcome(schema: &Schema, ty: &Ty, got: Result<Val, Error>) -> Result<Value, String> {
    match got {
        Ok(v) => Ok(to_ref(schema, ty, &v)),
        Err(Error::Fault(x)) => Err(format!("fault {}", x.name())),
        Err(Error::StatementLimit(_)) => Err("limit".into()),
        Err(e) => Err(format!("{e:?}")),
    }
}

fn expected(want: &R<Value>) -> Result<Value, String> {
    match want {
        Ok(v) => Ok(v.clone()),
        Err(Abort::Fault(x)) => Err(format!("fault {}", x.name())),
        Err(Abort::Limit(_)) => Err("limit".into()),
        Err(e) => Err(format!("{e:?}")),
    }
}

/// The store a body runs over and the morsel workers it runs on.
type Over<'a> = (
    &'a SimRuntime,
    &'a Arc<Store<SimRuntime>>,
    Option<&'a Arc<Scheduler<SimRuntime>>>,
);

/// Run `body` under the simulator over the store of a read-only transaction of a new database
/// of the fixture's declarations holding its rows, with `workers` morsel workers, beside the
/// transaction [`load::running_noise`] leaves running.
///
/// Precondition: `f` is [`planned`].
fn with_store<T>(seed: u64, f: &Fixture, workers: usize, body: impl FnOnce(Over<'_>) -> T) -> T {
    with_store_as(seed, f, workers, Running::Left, body)
}

/// Whether [`with_store_as`] leaves the transaction of [`load::running_noise`] running.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Running {
    Left,
    /// No transaction runs, so every committed noise version stays in the file.
    None,
}

fn with_store_as<T>(
    seed: u64,
    f: &Fixture,
    workers: usize,
    running: Running,
    body: impl FnOnce(Over<'_>) -> T,
) -> T {
    with_database(seed, f, workers, running, |_, over| body(over))
}

/// Run `body` as [`with_store_as`] does, over the database the store is of as well.
fn with_database<T>(
    seed: u64,
    f: &Fixture,
    workers: usize,
    running: Running,
    body: impl FnOnce(&crate::Database<SimRuntime>, Over<'_>) -> T,
) -> T {
    let mut out = None;
    let mut sim = loaded(seed, f);
    finished(&mut sim, |rt| {
        let db = load::open_with(&rt, f.decls(), &options());
        let _running =
            (running == Running::Left).then(|| load::running_noise(&db, &f.schema, &f.db));
        assert_eq!(
            f.catalog,
            db.testing().plan_catalog(),
            "the fixture plans over the database's catalog"
        );
        let reader = db.begin(&read_only()).expect("a transaction");
        let store = db
            .testing()
            .store(&reader)
            .expect("the transaction is open");
        let pool = (workers > 0).then(|| Scheduler::start(rt.clone(), workers));
        out = Some(body(&db, (&rt, &store, pool.as_ref())));
        if let Some(p) = pool {
            p.shutdown();
        }
    });
    out.expect("the run finished")
}

fn call(
    store: &Arc<Store<SimRuntime>>,
    grant: Arc<exec::Grant>,
    pool: Option<&Arc<Scheduler<SimRuntime>>>,
    options: XOptions,
) -> Call<SimRuntime> {
    Call {
        store: store.clone(),
        grant,
        pool: pool.cloned(),
        options,
        cancel: Arc::new(AtomicBool::new(false)),
        deadline: None,
    }
}

/// Run every case under the simulator. Returns each case's outcome, and how many tasks the
/// workers ran.
fn run_all(
    seed: u64,
    f: &Fixture,
    cases: &[Case],
    workers: usize,
    opts: XOptions,
) -> (Vec<Result<Value, String>>, usize) {
    with_store(seed, f, workers, |(rt, store, pool)| {
        let budget = Budget::new(1 << 30);
        let mut got = Vec::new();
        for c in cases {
            let grant = budget.admit(64 << 20).expect("a grant fits");
            let v = exec::execute(rt.clone(), c.bound(), call(store, grant, pool, opts));
            got.push(outcome(&f.schema, &c.ty, v));
        }
        assert_eq!(0, budget.granted(), "every grant returned to the budget");
        (got, pool.map_or(0, |p| p.ran()))
    })
}

/// Check that every case's outcome under the simulator is the reference interpreter's. Returns
/// how many tasks the workers ran.
fn check(seed: u64, f: &Fixture, cases: &[Case], workers: usize, opts: XOptions) -> usize {
    let (got, ran) = run_all(seed, f, cases, workers, opts);
    let mut bad = Vec::new();
    for (c, g) in cases.iter().zip(got) {
        let want = expected(&c.want);
        if want != g {
            bad.push(format!("{}:\n  want {want:?}\n  got  {g:?}", c.name));
        }
    }
    assert!(
        bad.is_empty(),
        "seed {seed}: {} of {} cases differ:\n{}",
        bad.len(),
        cases.len(),
        bad.join("\n")
    );
    ran
}

fn key(i: i64) -> Value {
    Value::key(Value::Int(i))
}

fn ident(f: &Fixture, n: u8) -> Value {
    f.reference(&uuid(n), &[]).expect("a key")
}

const QUERIES: &[&str] = &[
    "from i in identities select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub",
    "from i in identities, u in users where i.user == key u
     select { name: u.name, login: match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub }",
    "from g in identities.Google select g.gid",
    r#"from i in identities where match i | Google g -> g.gid == "root" | _ -> false select key i"#,
    r#"from i in identities.Google where i.gid == "root" select key i"#,
    "from i in identities select i.user.name",
    "from u in users select { name: u.name, n: count(i for i in u.<identities.user) }",
    "from i in identities group i by i.user into g select { user: g.key, n: count(x for x in g) }",
    "from o in orders group o by o.user into g select { user: g.key, total: sum(x.total for x in g) }",
    "from o in orders group o by o.user into g
     select { user: g.key, top: max(x.total for x in g), n: count(x for x in g where x.qty > 0) }",
    "from o in orders group o by o.user into g
     select { user: g.key, ids: from x in g select x.id, n: count(x for x in g) }",
    "from o in orders group o by o.user into g
     select { user: g.key, big: if count(x for x in g) > 1 then sum(x.total for x in g) else 0 }",
    "from u in users select u order by u.name desc",
    "from p in posts select p order by (p.created_at desc, key p) take 2",
    "from u in users
     select { user: u.name,
              latest: top 2 by (p.created_at desc, key p) (from p in posts where p.author == key u select p) }",
    "from o in orders where o.qty > 0 and o.total / o.qty > 10 select o.id",
    "from o in orders where o.total / o.qty > 10 select o.id",
    "from o in orders, u in users where o.total / o.qty > 10 and o.user == key u select u.name",
    "from o in orders, u in users where o.qty > 0 and o.total / o.qty > 10 and o.user == key u select u.name",
    "from u in users select { name: u.name, phones: from d in phones where d.owner == key u select d.seen }",
    "from r in (from u in users select { name: u.name, ds: u.<phones.owner }), d in r.ds
     select { name: r.name, seen: d.seen }",
    "from i in identities, u in users where key u == i.user select u.name",
    "from p in posts select p.author",
    "from u in users, i in u.<identities.user select { name: u.name, id: key i }",
    "from i in identities select match i | Google g -> g | Apple a -> a | EmailPass e -> e",
    "from i in identities.(EmailPass | Google) select match i | EmailPass e -> e.email | Google g -> g.gid",
    "from i in identities group i by tag(i) into g select { kind: g.key, n: count(x for x in g) }",
    "count(x for x in (from i in identities select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub))",
    "from u in users select { name: u.name, top: max(o.total for o in orders where o.user == key u) }",
    "from u in users select { name: u.name, n: count(s for s in u.<sessions.owner) }",
    "from o in orders select match checked_div(o.total, o.qty) | None -> 0 | Some { value: q } -> q",
    "from o in orders select { id: o.id, r: try (o.total / o.qty) }",
    "[1, 2] union (from o in orders select o.qty)",
    "from u in users where exists p in posts where p.author == key u select u.name",
    "from u in users where not exists p in posts where p.author == key u select u.name",
    "from f in follows, g in follows where f.dst == g.src and g.dst == f.src select { a: f.src, b: g.src }",
    "fix a = (from u in users select key u) union (from x in a where { k: x } != { k: me } select x) in a",
    "{ n: count(u for u in users), names: from u in users select u.name }",
    "if exists u in users where u.name == \"bo\" then count(p for p in posts) else 0",
    "from s in sessions select match s.owner | None -> 0 | Some { value: o } -> count(p for p in posts where p.author == o)",
    "from u in users, i in identities where key u == me and i.user == key u select i.id",
    "fix reach = (from f in follows where f.src == me select f.dst)
          union (from r in reach, f in follows where f.src == r select f.dst)
     in from u in users where exists r in reach where r == key u select u",
    "fix even = (from u in users where key u == me select key u)
                union (from o in odd, f in follows where f.src == o select f.dst),
         odd  = from v in even, f in follows where f.src == v select f.dst
     in from u in users where exists o in odd where o == key u select u",
    "fix reach = (from f in follows where f.src == me select f.dst)
               union (from r in reach, f in follows where f.src == r select f.dst)
     in from u in users where not exists r in reach where r == key u select u",
    "sum(o.total for o in orders where o.user == me)",
    "from o in orders select o.total * 2 - o.qty",
    "from u in users select u.name ++ \"!\"",
    "from o in orders where o.id > orders.key(2) select o.total",
    "from o in orders where o.id >= orders.key(2) and o.id < orders.key(4) select o.total",
    "from x in follows where x.src == me and x.dst > users.key(1) select x.dst",
    "from a in orders, b in orders where b.id > a.id and b.qty < a.qty select { a: a.id, b: b.id }",
    "from u in users select { u: u.name, out: from x in follows where x.src == key u and x.dst != me select x.dst }",
    "from u in users select count(o for o in orders where o.user == key u and o.total / o.qty > 10)",
    "from u in users select count(o for o in orders where o.user == key u and o.qty > 0 and o.total / o.qty > 10)",
    "exists o in orders where o.total / o.qty > 10",
    "exists p in posts where p.author == me",
    "top 0 by (o.total, key o) (from o in orders select o)",
    "top (0 - 1) by (o.total, key o) (from o in orders select o)",
    "from o in orders where orders.key(2) >= o.id select o.total",
    "from o in orders where orders.key(3) < o.id and o.qty > 0 select o.total",
    "from x in follows where x.src == users.key(2) and x.dst > users.key(2) select x.dst",
    "from u in users, o in orders where o.id <= orders.key(2) and o.user == key u select { u: u.name, o: o.id }",
    "from o in orders where o.id > orders.key(1) and o.id > orders.key(2) select o.id",
    "exists p in posts where p.author == users.key(1)",
    "exists p in posts where p.title == \"hello\"",
    "top 1 by (o.total / o.qty, key o) (from o in orders select o)",
    "from u in users select { u: u.name, out: from x in follows where x.src == key u and x.dst == key u select x.dst }",
    "from u in users select from p in posts where p.author == key u and p.title != u.name select p.title",
    "{ n: count(o for o in orders where o.user == me) }",
    "from u in users where u.name != \"cy\" select count(o for o in orders where o.qty > 0 and o.user == key u and o.total / o.qty > 10)",
    "from u in users, v in users, w in users where key v == key u and key v == users.key(999) select 1",
    "from u in users, v in users where key v == key u and key v == me select key u",
    "exists o in orders where o.qty > 0 and o.total / o.qty > 10",
];

fn running_cases(f: &Fixture) -> Vec<Case> {
    let mut out = Vec::new();
    for me in 1..=3 {
        for q in QUERIES {
            out.extend(cases_of(f, q, &[("me", "Key users", key(me))]));
        }
    }
    let probe = "from i in identities where i.id == k
                 select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub";
    for n in 1..=4 {
        out.extend(cases_of(f, probe, &[("k", "Key identities", ident(f, n))]));
    }
    out
}

#[test]
fn a_program_bound_without_a_value_for_its_parameter_is_refused_naming_it() {
    let f = running();
    let text = "from u in users where key u == me select u";
    for c in cases_of(&f, text, &[("me", "Key users", key(1))]) {
        assert_eq!(
            Error::Internal(Box::new(Broken::UnboundParam("me".to_owned()))),
            c.program.clone().bind(|_| None).unwrap_err(),
            "{}",
            c.name
        );
    }
}

#[test]
fn queries_over_heaps_and_indexes_agree_with_the_reference() {
    let f = running();
    let cases = running_cases(&f);
    check(1, &f, &cases, 0, XOptions::default());
}

#[test]
fn morsels_on_several_workers_agree_for_every_seed() {
    let f = running();
    let cases = running_cases(&f);
    let opts = XOptions {
        morsel_rows: 2,
        pack_pages: 0,
        ..XOptions::default()
    };
    let ran: usize = sweep(0..4)
        .map(|seed| check(seed, &f, &cases, 3, opts))
        .sum();
    assert!(ran > 0, "no worker ran a task");
}

const STOCK: &str = r"
type Item = { id: Key stock, g: I64, v: I64, name: Text }
relation stock : Item key id generate sequence
  index (g)
type Note = { id: Key notes, item: Key stock, label: Text }
relation notes : Note key id generate sequence
  index (item)
";

/// Returns a fixture whose heaps span many pages.
fn stock() -> Fixture {
    let mut f = Fixture::new(STOCK);
    let items: Vec<String> = (1..=300)
        .map(|i| {
            format!(
                r#"Item {{ id: stock.key({i}), g: {}, v: {}, name: "item{i}" }}"#,
                i % 7,
                (i * 37) % 101
            )
        })
        .collect();
    for chunk in items.chunks(40) {
        f.rows("stock", &format!("[{}]", chunk.join(", ")), &[]);
    }
    let notes: Vec<String> = (1..=150)
        .map(|i| {
            format!(
                r#"Note {{ id: notes.key({i}), item: stock.key({}), label: "n{i}" }}"#,
                (i * 3) % 300 + 1
            )
        })
        .collect();
    for chunk in notes.chunks(40) {
        f.rows("notes", &format!("[{}]", chunk.join(", ")), &[]);
    }
    planned(&f)
}

const STOCK_QUERIES: &[&str] = &[
    "from s in stock where s.v > 50 select s.id",
    "from s in stock group s by s.g into g select { g: g.key, n: count(x for x in g), t: sum(x.v for x in g) }",
    "from s in stock, n in notes where n.item == key s select { s: s.name, n: n.label }",
    "count(s for s in stock)",
    "from s in stock where s.g == 3 select s.v",
    "max(s.v for s in stock)",
    "from s in stock where 100 / (s.v - 50) > 1 select s.id",
    "from s in stock select s order by (s.v desc, key s) take 5",
    "from s in stock select { id: s.id, notes: from n in notes where n.item == key s select n.label }",
];

#[test]
fn morsels_of_heaps_of_many_pages_agree_for_every_seed() {
    let f = stock();
    let mut cases = Vec::new();
    for q in STOCK_QUERIES {
        cases.extend(cases_of(&f, q, &[]));
    }
    let opts = XOptions {
        morsel_rows: 16,
        range_pages: 2,
        ..XOptions::default()
    };
    let ran: usize = sweep(0..4)
        .map(|seed| check(seed, &f, &cases, 4, opts))
        .sum();
    assert!(ran > 0, "no worker ran a task");
}

/// Returns a grant of 64 MiB from a budget of its own.
fn roomy() -> Arc<exec::Grant> {
    Budget::new(1 << 30).admit(64 << 20).expect("a grant fits")
}

/// The reads that count the versions whose visibility they decide.
struct Counted {
    inner: Arc<dyn Reads>,
    seen: AtomicUsize,
}

impl Counted {
    fn take(&self) -> usize {
        self.seen.swap(0, Ordering::Relaxed)
    }
}

impl Reads for Counted {
    fn saw(&self, at: (Pid, HeapNo), writers: &Writers) -> Result<bool, Error> {
        self.seen.fetch_add(1, Ordering::Relaxed);
        self.inner.saw(at, writers)
    }

    fn scanning(&self, heap: HeapNo) -> Result<Option<ScanId>, SerializationFailure> {
        self.inner.scanning(heap)
    }

    fn scanned(&self, heap: HeapNo, scan: ScanId, read: Option<(StopsEarly, &[Pid])>) {
        self.inner.scanned(heap, scan, read);
    }

    fn probed(
        &self,
        index: Pid,
        lo: Bound<Vec<u8>>,
        hi: Bound<Vec<u8>>,
        ctors: BTreeSet<CtorId>,
    ) -> Result<(), SerializationFailure> {
        self.inner.probed(index, lo, hi, ctors)
    }

    fn fetched(&self, heap: HeapNo, tid: Tid) -> Result<(), SerializationFailure> {
        self.inner.fetched(heap, tid)
    }

    fn covers(&self, heap: HeapNo) -> bool {
        self.inner.covers(heap)
    }
}

/// Returns `store` with its reads counted.
fn counted(store: &Store<SimRuntime>) -> (Arc<Store<SimRuntime>>, Arc<Counted>) {
    let reads = Arc::new(Counted {
        inner: store.reads().clone(),
        seen: AtomicUsize::new(0),
    });
    (Arc::new(rereading(store, reads.clone())), reads)
}

/// Returns `store` reading through `reads`.
fn rereading(store: &Store<SimRuntime>, reads: Arc<dyn Reads>) -> Store<SimRuntime> {
    let history = store.history().clone();
    Store::over(
        store.pager().clone(),
        reads,
        store.bindings().clone(),
        store.listed(),
        history,
    )
    .expect("the store's bindings bind what it lists")
}

/// Returns how many versions a scan of `heap` reads.
fn versions(store: &Store<SimRuntime>, count: &Counted, heap: HeapId) -> usize {
    count.take();
    let listing = store.pages(heap).expect("a listing");
    store
        .scan(heap, listing, 1000, &roomy(), |_| {
            Ok(ControlFlow::Continue(()))
        })
        .expect("a scan");
    count.take()
}

#[test]
fn a_scan_passes_its_first_batch_before_it_reads_the_heap_s_last_page() {
    let f = stock();
    let heap = f.catalog.heap(f.rel("stock"), 0);
    let (total, first, stopped) = with_store(0, &f, 0, |(_, store, _)| {
        let (store, count) = counted(store);
        let total = versions(&store, &count, heap);
        let mut first = None;
        let listing = store.pages(heap).expect("a listing");
        store
            .scan(heap, listing, 7, &roomy(), |_| {
                first.get_or_insert(count.take());
                Ok(ControlFlow::Continue(()))
            })
            .expect("a scan");
        let listing = store.pages(heap).expect("a listing");
        count.take();
        store
            .scan(heap, listing, 7, &roomy(), |_| {
                Ok(ControlFlow::Break(StopsEarly::tested()))
            })
            .expect("a stopped scan");
        (total, first.expect("a batch"), count.take())
    });
    assert!(
        first < total,
        "{first} of {total} versions read before the first batch"
    );
    assert_eq!(first, stopped, "a scan stopped at its first batch");
}

/// Returns a store over the heaps and indexes of `store` whose reader holds back only `heaps`.
fn scoped(
    db: &crate::Database<SimRuntime>,
    store: &Store<SimRuntime>,
    heaps: &[HeapId],
) -> Store<SimRuntime> {
    let txns = db.tested_db().txns();
    let scope = heaps.iter().map(|h| store.stored(*h).heap.id().0).collect();
    let hold = txns.take(Scope::Heaps(scope)).expect("a snapshot");
    Store::safe(
        txns.reader(store.pager(), &hold, None),
        store.bindings().clone(),
    )
}

fn outside_scope(heap: HeapNo) -> Error {
    Error::Internal(Box::new(Broken::OutsideScope(heap)))
}

fn probed(
    store: &Store<SimRuntime>,
    index: IndexId,
    routes: &Routes,
) -> Result<Vec<exec::Named>, Error> {
    let grant = roomy();
    let bounds = ProbeBounds {
        grant: &grant,
        check: &|| Ok(()),
    };
    store.probe(index, &[], routes, bounds)
}

#[test]
fn a_store_reads_the_heaps_of_its_reader_s_scope_and_refuses_another() {
    let f = stock();
    let (stock, notes) = (
        f.catalog.heap(f.rel("stock"), 0),
        f.catalog.heap(f.rel("notes"), 0),
    );
    let (inside, outside, unscoped) =
        with_database(0, &f, 0, Running::None, |db, (_, store, _)| {
            let scoped = scoped(db, store, &[stock]);
            let rows = |heap: HeapId| {
                let mut n = 0;
                scoped
                    .pages(heap)
                    .and_then(|listing| {
                        scoped.scan(heap, listing, 1000, &roomy(), |b| {
                            n += b.len();
                            Ok(ControlFlow::Continue(()))
                        })
                    })
                    .map(|()| n)
            };
            (rows(stock), rows(notes), store.stored(notes).heap.id())
        });
    assert_eq!(Ok(300), inside);
    assert_eq!(Err(outside_scope(unscoped)), outside);
}

#[test]
fn a_store_probes_an_index_only_when_its_reader_covers_every_route_heap() {
    let f = stock();
    let (stock, notes) = (
        f.catalog.relation(f.rel("stock")),
        f.catalog.relation(f.rel("notes")),
    );
    let (inside, outside, unscoped) =
        with_database(0, &f, 0, Running::None, |db, (_, store, _)| {
            let scoped = scoped(db, store, &[stock.heaps[&0]]);
            let visible = |rel: &crate::plan::RelationDesc| -> Result<usize, Error> {
                let mut found = BTreeSet::new();
                for named in probed(&scoped, rel.key_index, &rel.heaps)? {
                    if let Some((tid, _)) = scoped.fetch(rel.heaps[&named.ctor], &named)? {
                        found.insert(tid);
                    }
                }
                Ok(found.len())
            };
            (
                visible(stock),
                visible(notes),
                store.stored(notes.heaps[&0]).heap.id(),
            )
        });
    assert_eq!(Ok(300), inside);
    assert_eq!(Err(outside_scope(unscoped)), outside);
}

#[test]
fn a_store_fetches_only_when_its_reader_covers_every_heap_of_the_relation() {
    let f = running();
    let rel = f.catalog.relation(f.rel("identities"));
    let (email, google, apple) = (rel.heaps[&0], rel.heaps[&1], rel.heaps[&2]);
    let (whole, part, unscoped) = with_database(0, &f, 0, Running::None, |db, (_, store, _)| {
        let found = probed(store, rel.key_index, &Routes::from([(0, email)])).expect("a probe");
        let named = found
            .iter()
            .find(|n| store.fetch(email, n).is_ok_and(|row| row.is_some()))
            .expect("an email identity");
        let fetched = |heaps: &[HeapId]| {
            scoped(db, store, heaps)
                .fetch(email, named)
                .map(|row| row.is_some())
        };
        (
            fetched(&[email, google, apple]),
            fetched(&[email, apple]),
            store.stored(google).heap.id(),
        )
    });
    assert_eq!(Ok(true), whole);
    assert_eq!(Err(outside_scope(unscoped)), part);
}

fn unlisted_heap(heap: HeapId) -> Error {
    Error::Internal(Box::new(Broken::UnlistedHeap(heap)))
}

#[test]
fn a_store_narrowed_to_a_plan_s_reads_refuses_a_heap_or_index_they_do_not_list() {
    let f = stock();
    let (stock, notes) = (
        f.catalog.relation(f.rel("stock")),
        f.catalog.relation(f.rel("notes")),
    );
    let listed = Arc::new(crate::plan::Reads {
        heaps: [stock.heaps[&0]].into(),
        indexes: [stock.key_index].into(),
    });
    let unbound = Arc::new(crate::plan::Reads {
        heaps: [HeapId(u64::MAX)].into(),
        indexes: BTreeSet::new(),
    });
    let (inside, heap, index, widened, missing) =
        with_database(0, &f, 0, Running::None, |_, (_, store, _)| {
            let narrowed = store
                .narrowed(&listed)
                .expect("the store lists every bound read");
            let rows = |heap: HeapId| {
                let mut n = 0;
                narrowed
                    .pages(heap)
                    .and_then(|listing| {
                        narrowed.scan(heap, listing, 1000, &roomy(), |b| {
                            n += b.len();
                            Ok(ControlFlow::Continue(()))
                        })
                    })
                    .map(|()| n)
            };
            let everything = Arc::new(crate::plan::Reads {
                heaps: [stock.heaps[&0], notes.heaps[&0]].into(),
                indexes: BTreeSet::new(),
            });
            (
                rows(stock.heaps[&0]),
                rows(notes.heaps[&0]),
                probed(&narrowed, notes.key_index, &Routes::new()).map(|n| n.len()),
                narrowed.narrowed(&everything).map(drop),
                store.narrowed(&unbound).map(drop),
            )
        });
    assert_eq!(Ok(300), inside);
    assert_eq!(Err(unlisted_heap(notes.heaps[&0])), heap);
    assert_eq!(
        Err(Error::Internal(Box::new(Broken::UnlistedIndex(
            notes.key_index
        )))),
        index
    );
    assert_eq!(Err(unlisted_heap(notes.heaps[&0])), widened);
    assert_eq!(
        Err(Error::Internal(Box::new(Broken::UnboundHeap(HeapId(
            u64::MAX
        ))))),
        missing
    );
}

#[test]
fn a_store_fetches_only_when_it_lists_every_heap_of_the_relation() {
    let f = running();
    let rel = f.catalog.relation(f.rel("identities"));
    let (email, google, apple) = (rel.heaps[&0], rel.heaps[&1], rel.heaps[&2]);
    let (whole, part) = with_database(0, &f, 0, Running::None, |_, (_, store, _)| {
        let found = probed(store, rel.key_index, &Routes::from([(0, email)])).expect("a probe");
        let named = found
            .iter()
            .find(|n| store.fetch(email, n).is_ok_and(|row| row.is_some()))
            .expect("an email identity");
        let fetched = |heaps: &[HeapId]| {
            let listed = Arc::new(crate::plan::Reads {
                heaps: heaps.iter().copied().collect(),
                indexes: [rel.key_index].into(),
            });
            let narrowed = store.narrowed(&listed).expect("the store lists every read");
            narrowed.fetch(email, named).map(|row| row.is_some())
        };
        (fetched(&[email, google, apple]), fetched(&[email, apple]))
    });
    assert_eq!(Ok(true), whole);
    assert_eq!(Err(unlisted_heap(google)), part);
}

#[test]
fn an_execution_over_a_store_that_does_not_list_its_program_s_reads_is_refused() {
    let f = stock();
    let (stock, notes) = (
        f.catalog.heap(f.rel("stock"), 0),
        f.catalog.heap(f.rel("notes"), 0),
    );
    let cases = cases_of(&f, "count(n for n in notes)", &[]);
    assert!(!cases.is_empty(), "the text has cases");
    let listed = Arc::new(crate::plan::Reads {
        heaps: [stock].into(),
        indexes: BTreeSet::new(),
    });
    let got = with_database(0, &f, 0, Running::None, |_, (rt, store, _)| {
        let narrowed = Arc::new(store.narrowed(&listed).expect("the store lists stock"));
        cases
            .iter()
            .map(|c| {
                let call = call(&narrowed, roomy(), None, XOptions::default());
                exec::execute(rt.clone(), c.bound(), call)
            })
            .collect::<Vec<_>>()
    });
    for (c, got) in cases.iter().zip(got) {
        assert_eq!(Err(unlisted_heap(notes)), got, "{}", c.name);
    }
}

#[test]
fn an_exists_that_stops_early_reads_fewer_versions_than_the_heap_holds() {
    let f = stock();
    let heap = f.catalog.heap(f.rel("stock"), 0);
    let cases: Vec<Case> = cases_of(&f, "exists s in stock where s.v > 50", &[])
        .into_iter()
        .filter(|c| !c.name.ends_with("with none") && !c.name.ends_with("with early_stop off"))
        .collect();
    let opts = XOptions {
        morsel_rows: 8,
        ..XOptions::default()
    };
    let (total, got) = with_store(0, &f, 0, |(rt, store, _)| {
        let (store, count) = counted(store);
        let total = versions(&store, &count, heap);
        let budget = Budget::new(1 << 30);
        let got = cases
            .iter()
            .map(|c| {
                let g = budget.admit(64 << 20).expect("a grant fits");
                let v = exec::execute(rt.clone(), c.bound(), call(&store, g, None, opts));
                (c.name.clone(), v, count.take())
            })
            .collect::<Vec<_>>();
        (total, got)
    });
    for (name, v, read) in got {
        assert_eq!(Ok(Val::bool(true)), v, "{name}");
        assert!(
            read < total,
            "{name}: read {read} of the heap's {total} versions"
        );
    }
}

/// Returns, for each plan of an `exists` over `stock` deciding at item `deciding`, whether the
/// plan stops early and whether its transaction and a writer of item 250 that reads what that
/// transaction writes fail with a serialization failure, as [`skew`] runs them.
fn stopped_scan_beside_a_writer_of_its_row(
    deciding: u32,
    workers: usize,
    opts: XOptions,
) -> Vec<(bool, bool)> {
    let f = stock();
    let exists = format!(r#"exists s in stock where s.name == "item{deciding}""#);
    let cases = cases_of(&f, &exists, &[]);
    cases
        .iter()
        .map(|c| {
            let mut out = None;
            let mut sim = loaded(0, &f);
            finished(&mut sim, |rt| {
                let db = load::open_with(&rt, f.decls(), &options());
                let pool = (workers > 0).then(|| Scheduler::start(rt.clone(), workers));
                let reader = db.begin(&TxnOptions::default()).expect("a transaction");
                let writer = db.begin(&TxnOptions::default()).expect("a transaction");
                let store = db
                    .testing()
                    .store(&reader)
                    .expect("the transaction is open");
                let v = exec::execute(rt, c.bound(), call(&store, roomy(), pool.as_ref(), opts));
                drop(store);
                assert_eq!(Ok(Val::bool(true)), v, "{}", c.name);
                out = Some((c.rewrites.early_stop, skew(reader, writer)));
                if let Some(p) = pool {
                    p.shutdown();
                }
            });
            out.expect("the run finished")
        })
        .collect()
}

/// Have `writer` read item 1 and write item 250, the deciding row, and `reader` write item 1,
/// each by its key, then commit `reader` and then `writer`. Returns whether either failed with
/// a serialization failure.
fn skew(
    mut reader: crate::AdHocTransaction<'_, SimRuntime>,
    mut writer: crate::AdHocTransaction<'_, SimRuntime>,
) -> bool {
    let touch = |key: u32| {
        format!(
            "update s in app::stock where key s == app::stock.key({key}) set {{ s with v: s.v + 1 }}"
        )
    };
    let none = TxnCallOptions::default();
    let wrote = writer
        .query_text(
            "from s in app::stock where key s == app::stock.key(1) select s.v",
            &[],
            &none,
        )
        .and_then(|_| reader.query_text(&touch(1), &[], &none))
        .and_then(|_| writer.query_text(&touch(250), &[], &none));
    let ended = wrote
        .map(drop)
        .and_then(|()| reader.commit(None).map(drop))
        .and_then(|()| writer.commit(None).map(drop));
    match ended {
        Ok(()) => false,
        Err(e) if e.is_serialization() => true,
        Err(e) => panic!("a write or commit fails by {e:?}"),
    }
}

#[test]
fn a_stopped_scan_conflicts_with_a_writer_of_its_deciding_row() {
    let opts = XOptions {
        morsel_rows: 16,
        ..XOptions::default()
    };
    let got = stopped_scan_beside_a_writer_of_its_row(250, 0, opts);
    assert!(got.iter().any(|(stops, _)| *stops), "{got:?}");
    assert!(
        got.iter()
            .filter(|(stops, _)| *stops)
            .all(|(_, failed)| *failed),
        "{got:?}"
    );
}

#[test]
fn a_stopped_scan_over_ranges_conflicts_with_a_writer_of_its_deciding_row() {
    let opts = XOptions {
        morsel_rows: 16,
        range_pages: 1,
        ..XOptions::default()
    };
    let got = stopped_scan_beside_a_writer_of_its_row(250, 3, opts);
    assert!(got.iter().any(|(stops, _)| *stops), "{got:?}");
    assert!(
        got.iter()
            .filter(|(stops, _)| *stops)
            .all(|(_, failed)| *failed),
        "{got:?}"
    );
}

#[test]
fn a_mistyped_value_is_an_internal_error_in_batches() {
    let f = running();
    let users = f.rel("users");
    let row: Vec<FieldType> = row_ty(&f.schema, users, 0)
        .iter()
        .map(|(_, t)| field_type(&f.schema, t))
        .collect();
    let schema = batch::Schema::of_heap(&row);
    let text = |t: &str| Val::Text(t.into());
    let set = |xs: Vec<Val>| Val::Set(Arc::new(xs.into_iter().collect()));
    for v in [
        Val::Int(1),
        set(vec![Val::Int(1)]),
        set(vec![Val::Record(Arc::from([]))]),
        set(vec![Val::Record(vec![text("a"); row.len()].into())]),
    ] {
        assert!(
            matches!(
                exec::batches(&v, &schema, Rows::Fields, 2),
                Err(Error::Internal(_))
            ),
            "{v:?}"
        );
    }
}

#[test]
fn a_call_of_a_wasm_function_is_refused_before_it_reaches_the_executor_and_a_host_one_plans() {
    let f = Fixture::new(
        "type User = { id: Key users, name: Text }
         relation users : User key id generate sequence
         fn slugify (t: Text) -> Text = host \"slugify\" version \"1.0\" deterministic
         fn shout (t: Text) -> Text = wasm \"text.wasm\" export \"shout\"",
    );
    for (q, refused) in [
        ("from u in users select slugify(u.name)", None),
        ("from u in users select shout(u.name)", Some("shout")),
    ] {
        let (n, t) = f.normal(q, &[]);
        let got = plan(
            &f.schema,
            &f.normal,
            &f.catalog,
            n.valid(),
            t,
            PlanOptions::default(),
        )
        .map(|_| ());
        assert_eq!(
            refused.map_or(Ok(()), |name| Err(PlanError::UserCode(name.into()))),
            got,
            "{q}"
        );
    }
}

/// A group whose aggregates fuse into its sink. Without the fusion each group record holds its
/// rows, and the records together exceed `SPILL_GRANT`.
const GROUP_SPILLS: &str = "from s in stock group s by s.g into g
     select { g: g.key, n: count(x for x in g), t: sum(x.v for x in g) }";

const SPILL_GRANT: usize = 32 << 10;

/// Run every case under the simulator with a grant of `grant` bytes, and return each case's
/// outcome with the bytes it spilled.
fn run_spilling(
    seed: u64,
    f: &Fixture,
    cases: &[Case],
    workers: usize,
    opts: XOptions,
    grant: usize,
) -> Vec<(Result<Value, String>, usize)> {
    with_store(seed, f, workers, |(rt, store, pool)| {
        let budget = Budget::new(1 << 30);
        let mut got = Vec::new();
        for c in cases {
            let g = budget.admit(grant).expect("a grant fits");
            let v = exec::execute(rt.clone(), c.bound(), call(store, g.clone(), pool, opts));
            got.push((outcome(&f.schema, &c.ty, v), g.spilled()));
        }
        assert_eq!(0, budget.granted(), "every grant returned to the budget");
        got
    })
}

/// Queries over `stock` that stop early or probe a range of its key index.
const STOP_RANGE_QUERIES: &[&str] = &[
    "exists s in stock where s.v > 50",
    "exists s in stock where s.v > 500",
    "exists s in stock where s.id > stock.key(10) and s.v > 50",
    "from s in stock where s.id > stock.key(40) and s.id <= stock.key(260) select { id: s.id, name: s.name }",
    "count(s for s in stock where s.id >= stock.key(100))",
    "top 0 by (s.v, key s) (from s in stock select s)",
];

#[test]
fn early_stops_and_range_probes_under_a_small_grant_agree_with_the_reference() {
    let f = stock();
    let mut cases = Vec::new();
    for q in STOP_RANGE_QUERIES {
        cases.extend(cases_of(&f, q, &[]));
    }
    let opts = XOptions {
        morsel_rows: 8,
        range_pages: 2,
        pack_pages: 0,
        ..XOptions::default()
    };
    for seed in sweep(0..2) {
        let got = run_spilling(seed, &f, &cases, 3, opts, SPILL_GRANT);
        for (c, (g, _)) in cases.iter().zip(got) {
            assert_eq!(expected(&c.want), g, "seed {seed}: {}", c.name);
        }
    }
}

const BUILD_SPILLS: &str =
    "from s in stock, n in notes where n.item == key s select { s: s.id, n: n.id }";

/// Run every case under the simulator with a grant of `SPILL_GRANT` bytes, every write to a
/// spill file failing with `fails` when it is given, and return each case's error.
fn spill_errors(
    f: &Fixture,
    cases: &[Case],
    opts: XOptions,
    fails: Option<IoError>,
) -> Vec<Option<Error>> {
    with_store(0, f, 0, |(rt, store, _)| {
        if let Some(e) = fails {
            let spills = Match::kind(OpKind::Write).prefixed("spill/");
            rt.plan(Plan::new().rule(spills, Action::Fail(e)));
        }
        let budget = Budget::new(1 << 30);
        cases
            .iter()
            .map(|c| {
                let g = budget.admit(SPILL_GRANT).expect("a grant fits");
                exec::execute(rt.clone(), c.bound(), call(store, g, None, opts)).err()
            })
            .collect()
    })
}

#[test]
fn a_spill_file_that_fails_its_writes_fails_the_call_with_the_io_error() {
    let f = stock();
    let cases = cases_of(&f, BUILD_SPILLS, &[]);
    let opts = XOptions {
        morsel_rows: 16,
        ..XOptions::default()
    };
    let got = spill_errors(&f, &cases, opts, Some(IoError::Failed));
    for (c, e) in cases.iter().zip(got) {
        assert_eq!(Some(Error::Spill(IoError::Failed)), e, "{}", c.name);
    }
}

#[test]
fn a_spill_onto_a_full_device_is_rejected_for_its_memory_grant() {
    let f = stock();
    let cases = cases_of(&f, BUILD_SPILLS, &[]);
    let opts = XOptions {
        morsel_rows: 16,
        ..XOptions::default()
    };
    let got = spill_errors(&f, &cases, opts, Some(IoError::Full));
    for (c, e) in cases.iter().zip(got) {
        assert_eq!(
            Some(Error::StatementLimit(Exceeded::Memory {
                limit: SPILL_GRANT
            })),
            e,
            "{}",
            c.name
        );
    }
}

/// Returns a fixture of 200 items, 120 of them in the group `big` and the rest in groups 1 to 5.
fn one_big_group(big: i64) -> Fixture {
    let mut f = Fixture::new(STOCK);
    let items: Vec<String> = (1..=200)
        .map(|i| {
            let g = if i <= 120 { big } else { i % 5 + 1 };
            format!(r#"Item {{ id: stock.key({i}), g: {g}, v: {i}, name: "item{i}" }}"#)
        })
        .collect();
    for chunk in items.chunks(40) {
        f.rows("stock", &format!("[{}]", chunk.join(", ")), &[]);
    }
    planned(&f)
}

/// Returns, for each grant from 4 KiB to 64 KiB in steps of 1 KiB, whether each group-holding
/// case of `GROUP_SPILLS` over `f` stays within it.
fn groups_fit(f: &Fixture) -> Vec<(usize, String, bool)> {
    let cases: Vec<Case> = cases_of(f, GROUP_SPILLS, &[])
        .into_iter()
        .filter(|c| c.name.ends_with("with none") || c.name.ends_with("with group_agg off"))
        .collect();
    let opts = XOptions {
        morsel_rows: 16,
        ..XOptions::default()
    };
    with_store(0, f, 0, |(rt, store, _)| {
        let budget = Budget::new(1 << 30);
        let mut out = Vec::new();
        for grant in (4..=64).map(|k| k << 10) {
            for c in &cases {
                let g = budget.admit(grant).expect("a grant fits");
                let v = exec::execute(rt.clone(), c.bound(), call(store, g, None, opts));
                match v {
                    Ok(_) | Err(Error::StatementLimit(Exceeded::Memory { .. })) => {}
                    Err(e) => panic!("{} under a grant of {grant}: {e:?}", c.name),
                }
                out.push((grant, c.name.clone(), v.is_ok()));
            }
        }
        out
    })
}

#[test]
fn whether_a_grouping_fits_its_grant_does_not_depend_on_where_its_largest_group_sorts() {
    let first = groups_fit(&one_big_group(0));
    let last = groups_fit(&one_big_group(9));
    assert!(first.iter().any(|(_, _, ok)| *ok), "no grant fits");
    assert!(first.iter().any(|(_, _, ok)| !*ok), "every grant fits");
    for ((grant, name, a), (_, _, b)) in first.iter().zip(&last) {
        assert_eq!(
            a, b,
            "{name} under a grant of {grant}: fits with its largest group first {a}, last {b}"
        );
    }
}
