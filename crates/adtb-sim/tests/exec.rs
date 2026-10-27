mod plan {
    pub use adtb::plan::*;
}

#[path = "../../adtb/src/plan/tests/fixture.rs"]
#[expect(dead_code, reason = "this binary uses part of the planner fixture")]
mod fixture;

use std::collections::BTreeSet;
use std::ops::{Bound, ControlFlow};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use adtb::batch;
use adtb::exec::{
    self, Budget, Call, Error, Exceeded, Options as XOptions, Program, Reads, Rows, Scheduler,
    Store, Val,
};
use adtb::heap::{FieldType, Header, HeapNo};
use adtb::key::{CtorId, Tid, Value as KValue};
use adtb::mvcc::{SerializationFailure, Xid};
use adtb::plan::{Error as PlanError, HeapId, Options as PlanOptions, plan};
use adtb::runtime::IoError;
use adtb::storage::{PageError, Pid};
use adtb_lang::kernel::Var;
use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;
use adtb_reference::{Abort, R, Value};
use adtb_sim::{Config, Faults, Sim, SimRuntime};
use adtb_testkit::uuid;

use fixture::{Fixture, running, variants};

#[expect(
    dead_code,
    reason = "this binary reads a loaded database and binds none for writes"
)]
mod support;

use support::harness::finished;
use support::*;

fn world(f: &Fixture) -> World<'_> {
    World {
        schema: &f.schema,
        normal: &f.normal,
        catalog: &f.catalog,
        db: &f.db,
    }
}

/// One plan to run, with what the reference interpreter says it computes.
struct Case {
    name: String,
    program: Arc<Program>,
    params: Vec<(Var, Val)>,
    ty: Ty,
    want: R<Value>,
}

/// Returns a case per rewrite variant of ad hoc `text` over `params`, each given as name, type
/// and value.
fn cases_of(f: &Fixture, text: &str, params: &[(&str, &str, Value)]) -> Vec<Case> {
    let types: Vec<(&str, &str)> = params.iter().map(|(n, t, _)| (*n, *t)).collect();
    let (n, t) = f.normal(text, &types);
    let bound: Vec<(Var, Value)> = params
        .iter()
        .map(|(name, _, v)| (n.params[*name], v.clone()))
        .collect();
    let vars: Vec<Var> = bound.iter().map(|(v, _)| *v).collect();
    let want = f.reference(text, params);
    let ty = n.terms.ty(t).clone();
    variants()
        .into_iter()
        .map(|(name, rewrites)| {
            let p = plan(
                &f.schema,
                &f.normal,
                &f.catalog,
                &n.terms,
                t,
                &vars,
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

/// Runs `body` under the simulator over the fixture's rows in heaps and indexes, with
/// `workers` morsel workers.
fn with_store<T>(
    seed: u64,
    f: &Fixture,
    workers: usize,
    body: impl FnOnce(&SimRuntime, &Arc<Store<SimRuntime>>, Option<&Arc<Scheduler<SimRuntime>>>) -> T,
) -> T {
    let mut out = None;
    let mut sim = Sim::new(seed, Config::default());
    finished(&mut sim, |rt| {
        let store = Db::load(&rt, &world(f)).store(&rt);
        let pool = (workers > 0).then(|| Scheduler::start(rt.clone(), workers));
        out = Some(body(&rt, &store, pool.as_ref()));
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

/// Runs every case under the simulator and returns each case's outcome, and how many tasks the
/// workers ran.
fn run_all(
    seed: u64,
    f: &Fixture,
    cases: &[Case],
    workers: usize,
    opts: XOptions,
) -> (Vec<Result<Value, String>>, usize) {
    with_store(seed, f, workers, |rt, store, pool| {
        let budget = Budget::new(1 << 30);
        let mut got = Vec::new();
        for c in cases {
            let grant = budget.admit(64 << 20).expect("a grant fits");
            let v = exec::execute(
                rt.clone(),
                c.program.clone(),
                call(store, grant, pool, opts),
                &c.params,
            );
            got.push(outcome(&f.schema, &c.ty, v));
        }
        assert_eq!(0, budget.granted(), "every grant returned to the budget");
        (got, pool.map_or(0, |p| p.ran()))
    })
}

/// Checks that every case's outcome under the simulator is the reference interpreter's. Returns
/// how many tasks the workers ran.
#[expect(
    clippy::print_stderr,
    reason = "EXEC_SHOW prints a sample of results on stderr"
)]
fn check(seed: u64, f: &Fixture, cases: &[Case], workers: usize, opts: XOptions) -> usize {
    let (got, ran) = run_all(seed, f, cases, workers, opts);
    if std::env::var("EXEC_SHOW").is_ok() {
        for (c, g) in cases.iter().zip(&got).step_by(11) {
            eprintln!("{} => {g:?}", c.name);
        }
    }
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
    f
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

/// Returns the rows of every batch `scan` passes.
fn scanned_rows(
    scan: impl FnOnce(
        &mut dyn FnMut(batch::Batch) -> Result<ControlFlow<()>, Error>,
    ) -> Result<(), Error>,
) -> Vec<Vec<KValue>> {
    let mut rows = Vec::new();
    scan(&mut |b| {
        rows.extend((0..b.len()).map(|i| b.row(i)));
        Ok(ControlFlow::Continue(()))
    })
    .expect("a scan");
    rows
}

#[test]
fn scans_of_ranges_of_a_heap_s_pages_read_what_a_scan_of_the_heap_reads() {
    let f = stock();
    let heap = f.catalog.heap(f.rel("stock"), 0);
    let (whole, ranges, pages) = with_store(0, &f, 0, |_, store, _| {
        let pages = store.pages(heap).expect("the heap's pages");
        let whole = scanned_rows(|each| store.scan(heap, 1000, each));
        let ranges: Vec<Vec<KValue>> = pages
            .chunks(1)
            .flat_map(|r| scanned_rows(|each| store.scan_pages(heap, r, 7, each)))
            .collect();
        (whole, ranges, pages.len())
    });
    assert!(pages > 1, "the heap spans {pages} pages");
    assert_eq!(300, whole.len());
    assert_eq!(whole, ranges);
}

/// Reads that count the versions whose visibility they decide.
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
    fn sees(&self, h: &Header) -> Result<bool, PageError> {
        self.seen.fetch_add(1, Ordering::Relaxed);
        self.inner.sees(h)
    }

    fn scanned(&self, heap: HeapNo) -> Result<(), SerializationFailure> {
        self.inner.scanned(heap)
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

    fn depends(&self, writer: Xid) -> Result<(), Error> {
        self.inner.depends(writer)
    }
}

/// Returns `store` with its reads counted.
fn counted(store: &Store<SimRuntime>) -> (Arc<Store<SimRuntime>>, Arc<Counted>) {
    let reads = Arc::new(Counted {
        inner: store.reads.clone(),
        seen: AtomicUsize::new(0),
    });
    let store = Store {
        pager: store.pager.clone(),
        reads: reads.clone(),
        heaps: store.heaps.clone(),
        indexes: store.indexes.clone(),
        history: store.history.clone(),
        past: None,
    };
    (Arc::new(store), reads)
}

/// Returns how many versions a scan of `heap` reads.
fn versions(store: &Store<SimRuntime>, count: &Counted, heap: HeapId) -> usize {
    count.take();
    store
        .scan(heap, 1000, |_| Ok(ControlFlow::Continue(())))
        .expect("a scan");
    count.take()
}

#[test]
fn a_scan_passes_its_first_batch_before_it_reads_the_heap_s_last_page() {
    let f = stock();
    let heap = f.catalog.heap(f.rel("stock"), 0);
    let (total, first, stopped) = with_store(0, &f, 0, |_, store, _| {
        let (store, count) = counted(store);
        let total = versions(&store, &count, heap);
        let mut first = None;
        store
            .scan(heap, 7, |_| {
                first.get_or_insert(count.take());
                Ok(ControlFlow::Continue(()))
            })
            .expect("a scan");
        count.take();
        store
            .scan(heap, 7, |_| Ok(ControlFlow::Break(())))
            .expect("a stopped scan");
        (total, first.expect("a batch"), count.take())
    });
    assert!(
        first < total,
        "{first} of {total} versions read before the first batch"
    );
    assert_eq!(first, stopped, "a scan stopped at its first batch");
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
    let (total, got) = with_store(0, &f, 0, |rt, store, _| {
        let (store, count) = counted(store);
        let total = versions(&store, &count, heap);
        let budget = Budget::new(1 << 30);
        let got = cases
            .iter()
            .map(|c| {
                let g = budget.admit(64 << 20).expect("a grant fits");
                let v = exec::execute(
                    rt.clone(),
                    c.program.clone(),
                    call(&store, g, None, opts),
                    &[],
                );
                (c.name.clone(), v, count.take())
            })
            .collect::<Vec<_>>();
        (total, got)
    });
    for (name, v, read) in got {
        assert_eq!(Ok(Val::bool(true)), v, "{name}");
        assert!(
            read < total / 2,
            "{name}: read {read} of the heap's {total} versions"
        );
    }
}

#[test]
fn a_scan_of_a_heap_larger_than_its_grant_runs_within_the_grant() {
    let f = stock_of(2000);
    let mut cases = Vec::new();
    for q in [
        "count(s for s in stock)",
        "exists s in stock where s.v > 500",
    ] {
        cases.extend(cases_of(&f, q, &[]));
    }
    let opts = XOptions {
        morsel_rows: 16,
        ..XOptions::default()
    };
    let got = with_store(0, &f, 0, |rt, store, _| {
        let heap = f.catalog.heap(f.rel("stock"), 0);
        let rows: usize = scanned_rows(|each| store.scan(heap, 1000, each))
            .iter()
            .map(|r| r.iter().map(|v| format!("{v:?}").len()).sum::<usize>())
            .sum();
        assert!(rows > SPILL_GRANT, "the heap's rows print in {rows} bytes");
        let budget = Budget::new(1 << 30);
        cases
            .iter()
            .map(|c| {
                let g = budget.admit(SPILL_GRANT).expect("a grant fits");
                let v = exec::execute(
                    rt.clone(),
                    c.program.clone(),
                    call(store, g.clone(), None, opts),
                    &[],
                );
                (outcome(&f.schema, &c.ty, v), g.peak())
            })
            .collect::<Vec<_>>()
    });
    for (c, (g, peak)) in cases.iter().zip(got) {
        assert_eq!(expected(&c.want), g, "{}", c.name);
        assert!(peak <= SPILL_GRANT, "{}: peak {peak}", c.name);
    }
}

#[test]
fn a_query_past_its_grant_is_rejected_for_its_statement_limit() {
    let f = stock();
    let cases = cases_of(&f, "from s in stock select s", &[]);
    let c = &cases[0];
    let got = with_store(0, &f, 2, |rt, store, pool| {
        let budget = Budget::new(1 << 30);
        let small = budget.admit(4096).expect("a grant fits");
        let refused = exec::execute(
            rt.clone(),
            c.program.clone(),
            call(store, small, pool, XOptions::default()),
            &[],
        );
        let roomy = budget.admit(64 << 20).expect("a grant fits");
        let done = exec::execute(
            rt.clone(),
            c.program.clone(),
            call(store, roomy, pool, XOptions::default()),
            &[],
        );
        assert_eq!(0, budget.granted(), "every grant returned to the budget");
        (
            refused,
            done.map(|v| v.as_set().expect("a result is a set").len()),
        )
    });
    assert_eq!(
        Err(Error::StatementLimit(Exceeded::Memory { limit: 4096 })),
        got.0
    );
    assert_eq!(Ok(300), got.1);
}

#[test]
fn a_fixpoint_past_its_rounds_is_rejected_for_its_statement_limit() {
    let f = running();
    let text = "fix reach = (from f in follows where f.src == me select f.dst)
                union (from r in reach, f in follows where f.src == r select f.dst)
                in reach";
    let cases = cases_of(&f, text, &[("me", "Key users", key(1))]);
    let opts = XOptions {
        fix_rounds: 1,
        ..XOptions::default()
    };
    let got = with_store(0, &f, 0, |rt, store, pool| {
        let budget = Budget::new(1 << 30);
        cases
            .iter()
            .map(|c| {
                let g = budget.admit(1 << 20).unwrap();
                exec::execute(
                    rt.clone(),
                    c.program.clone(),
                    call(store, g, pool, opts),
                    &c.params,
                )
            })
            .collect::<Vec<_>>()
    });
    for (c, g) in cases.iter().zip(got) {
        assert_eq!(
            Err(Error::StatementLimit(Exceeded::Rounds { limit: 1 })),
            g,
            "{}",
            c.name
        );
    }
}

#[test]
fn a_cancelled_call_stops_at_a_morsel_boundary() {
    let f = stock();
    let cases = cases_of(&f, "from s in stock select s.name", &[]);
    let got = with_store(0, &f, 2, |rt, store, pool| {
        let budget = Budget::new(1 << 30);
        let mut c = call(
            store,
            budget.admit(1 << 20).unwrap(),
            pool,
            XOptions::default(),
        );
        c.cancel = Arc::new(AtomicBool::new(true));
        exec::execute(rt.clone(), cases[0].program.clone(), c, &[])
    });
    assert_eq!(Err(Error::Cancelled), got);
}

#[test]
fn a_set_of_rows_comes_back_as_batches_of_the_heap_schema() {
    let f = running();
    let cases = cases_of(&f, "from u in users select u", &[]);
    let users = f.rel("users");
    let row: Vec<FieldType> = Db::row_ty(&world(&f), users, 0)
        .iter()
        .map(|(_, t)| field_type(&f.schema, t))
        .collect();
    let schema = batch::Schema::of_heap(&row);
    let v = with_store(0, &f, 0, |rt, store, pool| {
        let budget = Budget::new(1 << 30);
        exec::execute(
            rt.clone(),
            cases[0].program.clone(),
            call(
                store,
                budget.admit(1 << 20).unwrap(),
                pool,
                XOptions::default(),
            ),
            &[],
        )
    })
    .expect("a value");
    let bs = exec::batches(&v, &schema, Rows::Fields, 2).expect("the users fill the batches");
    assert_eq!(
        vec![2, 1],
        bs.iter().map(batch::Batch::len).collect::<Vec<_>>()
    );
    let got: Vec<Vec<KValue>> = bs
        .iter()
        .flat_map(|b| (0..b.len()).map(|i| b.row(i)))
        .collect();
    let tys = Db::row_ty(&world(&f), users, 0);
    let want: Vec<Vec<KValue>> = f.db[&(users, 0)]
        .iter()
        .map(|r| {
            tys.iter()
                .map(|(n, t)| stored(&f.schema, t, r.field(n).unwrap()))
                .collect()
        })
        .collect();
    assert_eq!(want, got);
}

#[test]
fn a_mistyped_value_is_an_internal_error_in_batches() {
    let f = running();
    let users = f.rel("users");
    let row: Vec<FieldType> = Db::row_ty(&world(&f), users, 0)
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
            &n.terms,
            t,
            &[],
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

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the query of every primitive is listed inline; the list would have to be its own function"
)]
fn every_primitive_agrees_with_the_reference() {
    let f = running();
    let mut cases = Vec::new();
    let mut refused = Vec::new();
    for q in [
        "from u in users select lower(u.name)",
        "from u in users select upper(u.name)",
        "from u in users select u.name ++ \"\u{301}\"",
        "from o in orders select text(o.total)",
        "from o in orders select div(decimal(o.total, 2), decimal(o.qty + 1, 0), 2, HalfEven)",
        "from u in users select grapheme_len(u.name)",
        "from u in users select matches(u.name, \"^a\")",
        "from u in users select lower(\"ÀÉΣ\" ++ u.name)",
        "from u in users select lower(\"İ\" ++ u.name)",
        "from u in users select upper(u.name ++ \"ßŉ\")",
        "from u in users select fold(u.name ++ \"ẞΣς\")",
        "from u in users select fold(upper(u.name)) == fold(u.name)",
        "from u in users select grapheme_len(u.name ++ \"e\u{301}👍🏽\u{1f1e8}\u{1f1e6}\")",
        "from u in users select matches(u.name, \"(?i)^A\")",
        "from u in users select matches(u.name ++ \"\\n\", \"n$\")",
        "from u in users select matches(u.name ++ \"\\nx\", \"(?m)n$\")",
        "from u in users select matches(u.name ++ \"é\", \"^\\\\p{L}+$\")",
        "from u in users select matches(u.name ++ \"\\n\", \"(?s)n.\")",
        "from u in users select matches(u.name, \"(?U)a+\")",
        "from u in users select collate(u.name, \"en@cldr-49\")",
        "from u in users select collate(u.name ++ \"ö\", \"de-u-ks-level2@cldr-49\")",
        "from u in users select collate(u.name ++ \"ö\", \"sv@cldr-49\")",
        "from u in users select collate(upper(u.name) ++ \" 10\", \"en-u-kn-true-ka-shifted-kf-upper@cldr-49\")",
        "from u in users select collate(u.name, \"und-u-ks-level1-kc-true-kv-punct@cldr-49\")",
        "from u in users select utf8(bytes(u.name ++ \"é\"))",
        "from u in users select match Bytes.parse(\"0x65cc81\") | None -> None | Some b -> utf8(bytes(u.name) ++ b.value)",
        "from u in users select match Bytes.parse(\"0xff\") | None -> None | Some b -> utf8(bytes(u.name) ++ b.value)",
        "from u in users select \"e\" ++ \"\u{301}\" ++ u.name",
        "from o in orders select text(o.qty - 3)",
        "from o in orders select text(decimal(o.total, 2) * decimal(o.qty - 3, 1))",
        "from o in orders select text(f64(o.total) / 7.0)",
        "from o in orders select text(f64(o.total) * 1e300)",
        "from o in orders select text(f64(o.qty) / 1e10)",
        "from o in orders select text(f32(o.qty) / 3.0)",
        "from o in orders select text(f64(o.total) / f64(o.qty))",
        "from o in orders select text(f64(o.qty) / f64(o.qty))",
        "from o in orders select text(-f64(o.total) / f64(o.qty))",
        "from p in posts select text(p.created_at)",
        "from o in orders select text(micros(o.total * 36000000 - 1000))",
        "from o in orders select text(micros(-o.total * 1000000))",
        "from o in orders select text(days(o.qty) - months(o.qty))",
        "from u in users select text(uuid(\"0190A4B2-7C3E-7d4f-9a21-5b6c7d8e9f0\" ++ text(scalar_len(u.name))))",
        "from u in users select try uuid(u.name)",
        "from u in users select print(u)",
        "from i in identities select print(i)",
        "from i in identities select print(tag(i))",
        "from f in follows select print(key f)",
        "from o in orders select print({ total: o.total, user: o.user, half: decimal(o.total, 1) * 0.5 })",
        "from o in orders select print(checked_div(o.total, o.qty))",
        "from u in users select print(u.name ++ \"\\\"\\\\\\n\\r\\t\\u{7}é\")",
        "from p in posts select print(p.created_at - 7d)",
        "from o in orders select print(f64(o.qty) / 0.0)",
        "from u in users select User.parse(print(u))",
        "from i in identities select Identity.parse(print(i))",
        "from f in follows select (Key follows).parse(print(key f))",
        "from u in users select (Key users).parse(\"app::users.key(\" ++ text(scalar_len(u.name)) ++ \")\")",
        "from o in orders select I64.parse(\" -00\" ++ text(o.total) ++ \" \")",
        "from o in orders select I8.parse(text(o.total * 2))",
        "from o in orders select I64.parse(text(o.total) ++ \".0\")",
        "from o in orders select (Decimal 2).parse(text(o.total) ++ \".5\")",
        "from o in orders select (Decimal 2).parse(text(o.total) ++ \".555\")",
        "from o in orders select F64.parse(text(f64(o.total) / 7.0))",
        "from o in orders select F32.parse(text(o.total) ++ \"e37\")",
        "from o in orders select F64.parse(text(o.total) ++ \"e400\")",
        "from o in orders select F64.parse(\"-inf\") == Some { value: -f64(o.qty) / 0.0 }",
        "from u in users select Text.parse(print(u.name ++ \"\\\"\\n\"))",
        "from u in users select Text.parse(\"\\\"e\\\\u{301}\" ++ u.name ++ \"\\\"\")",
        "from u in users select Text.parse(u.name)",
        "from p in posts select Instant.parse(text(p.created_at))",
        "from o in orders select DateTime.parse(\"2026-10-25T09:30:0\" ++ text(o.qty) ++ \".5\")",
        "from o in orders select Date.parse(\"2024-02-2\" ++ text(o.qty + 5))",
        "from o in orders select Date.parse(\"+1000\" ++ text(o.qty) ++ \"-01-01\")",
        "from o in orders select Time.parse(\"23:59:5\" ++ text(o.qty) ++ \".25\")",
        "from o in orders select Duration.parse(text(o.qty * 60) ++ \"s\")",
        "from o in orders select Period.parse(\"P\" ++ text(o.qty) ++ \"M-\" ++ text(o.total) ++ \"D\")",
        "from o in orders select ZonedDateTime.parse(\"2026-03-29T01:3\" ++ text(o.qty) ++ \":00[Europe/Lisbon]\")",
        "from u in users select ZonedDateTime.parse(\"2026-03-29T01:30:00[\" ++ u.name ++ \"]\")",
        "from o in orders select (Set I64).parse(\"[\" ++ text(o.qty) ++ \", 1, 3]\")",
        "from o in orders select (Option I64).parse(\"Some {value:\" ++ text(o.qty) ++ \"}\")",
        "from o in orders select Bool.parse(if o.qty > 2 then \"true\" else \"False\")",
        "from o in orders select (Tag Identity).parse(if o.qty > 2 then \"Google\" else \"Nope\")",
        "from o in orders select try div(decimal(o.total, 2), decimal(o.qty, 1), 4, HalfUp)",
        "from o in orders select div(decimal(-o.total, 0), decimal(o.qty + 2, 0), 0, Floor)",
        "from o in orders select div(decimal(o.total, 0), decimal(o.qty + 2, 3), 1, HalfEven)",
        "from o in orders select try div(decimal(o.total * 10000000000000000, 0), decimal(o.qty + 1, 0), 20, Up)",
        "from o in orders select div(decimal(o.total, 6), decimal(o.qty + 1, 0), 2, HalfUp)",
        "from o in orders select i32(decimal(o.total, 0) * 0.15, HalfEven)",
        "from o in orders select i64(decimal(-o.total, 0) * 0.15, Ceiling)",
        "from o in orders select try i8(f64(o.total) * 1.5, Up)",
        "from o in orders select checked_i8(f64(o.total) * 1.25, HalfDown)",
        "from o in orders select checked_i64(f64(o.total) / f64(o.qty), Down)",
        "from o in orders select decimal(f64(o.total) / 3.0, 4, HalfEven)",
        "from o in orders select decimal(f32(o.qty) / 3.0, 6, Floor)",
        "from o in orders select checked_decimal(f64(o.total) / f64(o.qty), 2, Down)",
        "from o in orders select checked_decimal(f64(o.total) * 1e36, 2, Down)",
        "from o in orders select f64(decimal(o.total, 2) * 0.01)",
        "from o in orders select f32(decimal(o.total, 0) * 0.1)",
        "from p in posts select in_zone(p.created_at, \"Europe/Lisbon\")",
        "from p in posts select in_zone(p.created_at, \"US/Eastern\")",
        "from p in posts select try in_zone(p.created_at, \"europe/lisbon\")",
        "from p in posts select to_instant(in_zone(p.created_at, \"America/New_York\"))",
        "from p in posts select in_zone(p.created_at, \"Asia/Kolkata\") + 1h",
        "from p in posts select in_zone(p.created_at, \"Asia/Kolkata\") - 30min",
        "from o in orders select match DateTime.parse(\"2026-03-29T01:\" ++ text(o.total % 60 + 10) ++ \":00\") | None -> \"none\" | Some d -> print(zoned(d.value, \"Europe/Lisbon\", Clamp))",
        "from o in orders select match DateTime.parse(\"2026-03-29T01:\" ++ text(o.total % 60 + 10) ++ \":00\") | None -> \"none\" | Some d -> print(zoned(d.value, \"Europe/Lisbon\", Reject))",
        "from o in orders select match DateTime.parse(\"2026-10-25T01:\" ++ text(o.total % 60 + 10) ++ \":00\") | None -> \"none\" | Some d -> print(to_instant(zoned(d.value, \"Europe/Lisbon\", Clamp)))",
        "from o in orders select match DateTime.parse(\"2026-10-25T01:\" ++ text(o.total % 60 + 10) ++ \":00\") | None -> \"none\" | Some d -> print(zoned(d.value, \"Europe/Lisbon\", Clamp) + 1h)",
        "from o in orders select match Date.parse(\"2024-01-31\") | None -> \"none\" | Some d -> print(add_period(d.value, months(o.qty) + days(o.qty), Clamp))",
        "from o in orders select match Date.parse(\"2024-01-31\") | None -> \"none\" | Some d -> print(add_period(d.value, months(o.qty), Reject))",
        "from o in orders select match DateTime.parse(\"2024-03-31T12:00:00\") | None -> \"none\" | Some d -> print(add_period(d.value, -months(o.qty), Clamp))",
        "from o in orders select match ZonedDateTime.parse(\"2026-02-28T01:30:00[Europe/Lisbon]\") | None -> \"none\" | Some z -> print(add_period(z.value, days(o.qty + 26), Clamp))",
        "from o in orders select match ZonedDateTime.parse(\"2026-02-28T01:30:00[Europe/Lisbon]\") | None -> \"none\" | Some z -> print(add_period(z.value, days(o.qty + 26), Reject))",
        "from o in orders select span(o.qty, o.total)",
        "from o in orders select try span(o.total, o.qty)",
        "from o in orders select start(span(o.qty, o.total)) + end(span(o.qty, o.total))",
        "from o in orders select overlaps(span(o.qty, o.total), span(40, 60))",
        "from o in orders select covers(span(o.qty, o.total), 50)",
        "from o in orders select covers(span(0, 100), span(o.qty, o.total))",
        "from o in orders select print(span(o.qty, o.total))",
        "from o in orders select (Span I64).parse(\"span(\" ++ text(o.qty) ++ \", 3)\")",
        "from o in orders select (Span Date).parse(\"span(2024-01-0\" ++ text(o.qty + 1) ++ \", 2024-01-04)\")",
        "from p in posts select (Span Instant).parse(print(span(p.created_at, p.created_at + 1h)))",
        "from o in orders select Date.parse(\"+294247-01-\" ++ text(o.qty + 8))",
        "from o in orders select Date.parse(\"-290308-12-\" ++ text(o.qty + 19))",
        "from o in orders select Instant.parse(\"+294247-01-10T04:00:54.77580\" ++ text(o.qty + 5) ++ \"Z\")",
        "from o in orders select Instant.parse(\"-290308-12-21T19:59:05.22419\" ++ text(o.qty) ++ \"Z\")",
        "from o in orders select DateTime.parse(\"+294247-01-10T04:00:5\" ++ text(o.qty) ++ \".775807\")",
        "from o in orders select DateTime.parse(\"-290308-12-21T19:59:0\" ++ text(o.qty + 3) ++ \".224192\")",
        "from o in orders select ZonedDateTime.parse(\"+294247-01-10T04:00:5\" ++ text(o.qty) ++ \"[UTC]\")",
        "from o in orders select match Date.parse(\"+294247-01-10\") | None -> \"none\" | Some d -> print(add_period(d.value, days(o.qty - 2), Clamp))",
        "from o in orders select match Date.parse(\"-290308-12-21\") | None -> \"none\" | Some d -> print(add_period(d.value, -months(o.qty), Clamp))",
        "from o in orders select match DateTime.parse(\"+294247-01-09T05:00:00\") | None -> \"none\" | Some d -> print(add_period(d.value, days(o.qty), Clamp))",
        "from o in orders select match DateTime.parse(\"-290308-12-22T20:00:00\") | None -> \"none\" | Some d -> print(add_period(d.value, -days(o.qty), Clamp))",
        "from o in orders select match DateTime.parse(\"-290308-12-22T19:00:00\") | None -> \"none\" | Some d -> print(add_period(d.value, -days(o.qty), Clamp))",
        "from o in orders select match Instant.parse(\"+294247-01-10T04:00:54.775807Z\") | None -> \"none\" | Some i -> print(i.value - micros(o.qty))",
        "from o in orders select match Instant.parse(\"+294247-01-10T04:00:54.775807Z\") | None -> \"none\" | Some i -> print(i.value + micros(o.qty))",
        "from o in orders select match Instant.parse(\"-290308-12-21T19:59:05.224192Z\") | None -> \"none\" | Some i -> text(i.value + micros(o.qty - 2))",
        "from o in orders select match Instant.parse(\"+294247-01-10T04:00:54.775807Z\") | None -> \"none\" | Some i -> print(in_zone(i.value - micros(o.qty), \"Asia/Kolkata\"))",
        "from o in orders select match Instant.parse(\"-290308-12-21T19:59:05.224192Z\") | None -> \"none\" | Some i -> print(in_zone(i.value, \"America/New_York\") + micros(o.qty))",
    ] {
        match std::panic::catch_unwind(|| cases_of(&f, q, &[])) {
            Ok(cs) => cases.extend(cs),
            Err(_) => refused.push(q),
        }
    }
    if !cases.is_empty() {
        check(0, &f, &cases, 0, XOptions::default());
    }
    assert!(refused.is_empty(), "refused: {refused:#?}");
}

#[test]
fn every_program_records_the_type_of_its_root() {
    let f = running();
    for c in running_cases(&f) {
        assert_eq!(c.ty, c.program.ty, "{}", c.name);
    }
}

/// Queries over `stock` whose inputs exceed `SPILL_GRANT` while their results fit it.
const SPILL_QUERIES: &[&str] = &[
    "from s in stock, n in notes where n.item == key s select { s: s.id, n: n.id }",
    "count(x for x in (from s in stock, n in notes where n.item == key s select { s: s, n: n }))",
    "count(s for s in stock)",
    "max(s.name for s in stock)",
    "sum(s.v for s in stock)",
    "from s in stock select s order by (s.v desc, key s) take 5",
    "from s in stock select s order by (s.v, key s) take 40",
];

/// A group whose aggregates fuse into its sink. Without the fusion each group record holds its
/// rows, and the records together exceed `SPILL_GRANT`.
const GROUP_SPILLS: &str = "from s in stock group s by s.g into g
     select { g: g.key, n: count(x for x in g), t: sum(x.v for x in g) }";

const SPILL_GRANT: usize = 32 << 10;

/// Runs every case under the simulator with a grant of `grant` bytes, and returns each case's
/// outcome with the bytes it spilled.
fn run_spilling(
    seed: u64,
    f: &Fixture,
    cases: &[Case],
    workers: usize,
    opts: XOptions,
    grant: usize,
) -> Vec<(Result<Value, String>, usize)> {
    with_store(seed, f, workers, |rt, store, pool| {
        let budget = Budget::new(1 << 30);
        let mut got = Vec::new();
        for c in cases {
            let g = budget.admit(grant).expect("a grant fits");
            let v = exec::execute(
                rt.clone(),
                c.program.clone(),
                call(store, g.clone(), pool, opts),
                &c.params,
            );
            got.push((outcome(&f.schema, &c.ty, v), g.spilled()));
        }
        assert_eq!(0, budget.granted(), "every grant returned to the budget");
        got
    })
}

#[test]
fn queries_past_their_grant_spill_and_agree_with_the_reference() {
    let f = stock();
    let mut cases = Vec::new();
    for q in SPILL_QUERIES {
        cases.extend(cases_of(&f, q, &[]));
    }
    cases.extend(
        cases_of(&f, GROUP_SPILLS, &[])
            .into_iter()
            .filter(|c| !c.name.ends_with("with none") && !c.name.ends_with("with group_agg off")),
    );
    let opts = XOptions {
        morsel_rows: 16,
        ..XOptions::default()
    };
    let runs = std::iter::once((0, 0)).chain(sweep(0..1).map(|seed| (seed, 3)));
    for (seed, workers) in runs {
        {
            let got = run_spilling(seed, &f, &cases, workers, opts, SPILL_GRANT);
            let mut bad = Vec::new();
            for (c, (g, spilled)) in cases.iter().zip(got) {
                let want = expected(&c.want);
                if want != g || spilled == 0 {
                    bad.push(format!(
                        "{}: spilled {spilled}\n  want {want:?}\n  got  {g:?}",
                        c.name
                    ));
                }
            }
            assert!(
                bad.is_empty(),
                "seed {seed}, {workers} workers: {} of {} cases differ or did not spill:\n{}",
                bad.len(),
                cases.len(),
                bad.join("\n")
            );
        }
    }
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

#[test]
fn a_build_past_its_grant_with_no_spill_space_is_rejected_for_its_statement_limit() {
    let f = stock();
    let cases = cases_of(
        &f,
        "from s in stock, n in notes where n.item == key s select { s: s.id, n: n.id }",
        &[],
    );
    let opts = XOptions {
        morsel_rows: 16,
        spill_bytes: 0,
        ..XOptions::default()
    };
    let got = run_spilling(0, &f, &cases, 0, opts, SPILL_GRANT);
    for (c, (g, spilled)) in cases.iter().zip(got) {
        assert_eq!(Err("limit".to_string()), g, "{}", c.name);
        assert_eq!(0, spilled, "{}", c.name);
    }
}

const BUILD_SPILLS: &str =
    "from s in stock, n in notes where n.item == key s select { s: s.id, n: n.id }";

/// Runs every case under the simulator with a grant of `SPILL_GRANT` bytes and `faults` set
/// once the database is loaded, and returns each case's error.
fn spill_errors(f: &Fixture, cases: &[Case], opts: XOptions, faults: Faults) -> Vec<Option<Error>> {
    with_store(0, f, 0, |rt, store, _| {
        rt.set_faults(faults);
        let budget = Budget::new(1 << 30);
        cases
            .iter()
            .map(|c| {
                let g = budget.admit(SPILL_GRANT).expect("a grant fits");
                exec::execute(
                    rt.clone(),
                    c.program.clone(),
                    call(store, g, None, opts),
                    &c.params,
                )
                .err()
            })
            .collect()
    })
}

#[test]
fn a_spill_past_its_spill_bound_is_rejected_for_the_bound_and_the_grant() {
    let f = stock();
    let cases = cases_of(&f, BUILD_SPILLS, &[]);
    let opts = XOptions {
        morsel_rows: 16,
        spill_bytes: 4096,
        ..XOptions::default()
    };
    let got = spill_errors(&f, &cases, opts, Faults::default());
    for (c, e) in cases.iter().zip(got) {
        assert_eq!(
            Some(Error::StatementLimit(Exceeded::Spill {
                limit: 4096,
                grant: SPILL_GRANT
            })),
            e,
            "{}",
            c.name
        );
    }
}

#[test]
fn a_spill_file_that_fails_its_writes_fails_the_call_with_the_io_error() {
    let f = stock();
    let cases = cases_of(&f, BUILD_SPILLS, &[]);
    let opts = XOptions {
        morsel_rows: 16,
        ..XOptions::default()
    };
    let faults = Faults {
        fail_p: 1.0,
        ..Faults::default()
    };
    let got = spill_errors(&f, &cases, opts, faults);
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
    let faults = Faults {
        no_space: true,
        ..Faults::default()
    };
    let got = spill_errors(&f, &cases, opts, faults);
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
    f
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
    with_store(0, f, 0, |rt, store, _| {
        let budget = Budget::new(1 << 30);
        let mut out = Vec::new();
        for grant in (4..=64).map(|k| k << 10) {
            for c in &cases {
                let g = budget.admit(grant).expect("a grant fits");
                let v = exec::execute(
                    rt.clone(),
                    c.program.clone(),
                    call(store, g, None, opts),
                    &[],
                );
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

#[test]
fn a_call_removes_its_spill_file_when_it_ends() {
    let f = stock();
    let cases = cases_of(&f, "count(s for s in stock)", &[]);
    let opts = XOptions {
        morsel_rows: 16,
        ..XOptions::default()
    };
    let left = with_store(0, &f, 2, |rt, store, pool| {
        let budget = Budget::new(1 << 30);
        let g = budget.admit(SPILL_GRANT).expect("a grant fits");
        let v = exec::execute(
            rt.clone(),
            cases[0].program.clone(),
            call(store, g.clone(), pool, opts),
            &[],
        );
        assert_eq!(Ok(Val::Int(300)), v);
        assert!(g.spilled() > 0, "the call spilled");
        listed(rt, "spill")
    });
    assert_eq!(Vec::<String>::new(), left);
}

/// Returns the files in `dir` of the simulated file system.
fn listed(rt: &SimRuntime, dir: &str) -> Vec<String> {
    use adtb::runtime::{Done, Io, Op, Runtime, Threads};
    let mut io = rt.io();
    let t = io.submit(Op::List { dir: dir.into() });
    loop {
        match io.poll() {
            Some(c) if c.ticket == t => {
                return match c.result {
                    Ok(Done::Listed(paths)) => paths,
                    other => panic!("listing {dir} completed as {other:?}"),
                };
            }
            Some(_) => {}
            None => rt.park(),
        }
    }
}

#[test]
fn a_set_spilled_in_more_runs_than_one_merge_reads_agrees_with_the_reference() {
    let f = stock();
    let mut cases = Vec::new();
    for q in ["count(s for s in stock)", "max(s.name for s in stock)"] {
        cases.extend(cases_of(&f, q, &[]));
    }
    let opts = XOptions {
        morsel_rows: 2,
        ..XOptions::default()
    };
    let got = run_spilling(0, &f, &cases, 0, opts, 4 << 10);
    for (c, (g, spilled)) in cases.iter().zip(got) {
        assert_eq!(expected(&c.want), g, "{}", c.name);
        assert!(spilled > 64 * 4096, "{}: spilled {spilled}", c.name);
    }
}

/// Returns a fixture of `n` items over `n / 2` notes, its heaps spanning many pages.
fn stock_of(n: usize) -> Fixture {
    let mut f = Fixture::new(STOCK);
    let items: Vec<String> = (1..=n)
        .map(|i| {
            format!(
                r#"Item {{ id: stock.key({i}), g: {}, v: {}, name: "item{i}" }}"#,
                i % 7,
                (i * 37) % 101
            )
        })
        .collect();
    for chunk in items.chunks(200) {
        f.rows("stock", &format!("[{}]", chunk.join(", ")), &[]);
    }
    let notes: Vec<String> = (1..=n / 2)
        .map(|i| {
            format!(
                r#"Note {{ id: notes.key({i}), item: stock.key({}), label: "n{i}" }}"#,
                (i * 3) % n + 1
            )
        })
        .collect();
    for chunk in notes.chunks(200) {
        f.rows("notes", &format!("[{}]", chunk.join(", ")), &[]);
    }
    f
}

#[test]
#[ignore = "a benchmark: run with --ignored --no-capture"]
#[expect(
    clippy::disallowed_methods,
    reason = "a benchmark times the host, not the simulated clock"
)]
fn bench_scans_joins_and_groups() {
    let f = stock_of(4000);
    let mut programs = Vec::new();
    for q in [
        "from s in stock where s.v > 50 select s.id",
        "from s in stock group s by s.g into g select { g: g.key, n: count(x for x in g), t: sum(x.v for x in g) }",
        "count(x for x in (from s in stock, n in notes where n.item == key s select { s: s.id, n: n.id }))",
        "sum(s.v * 2 - s.g for s in stock)",
    ] {
        let c = cases_of(&f, q, &[]).into_iter().next().expect("a case");
        programs.push((q, c.program));
    }
    let configs: [(&str, usize, usize); 4] = [
        ("serial", 0, 8),
        ("4 workers, 1 page per task", 4, 1),
        ("4 workers, 8 pages per task", 4, 8),
        ("4 workers, one task per heap", 4, usize::MAX),
    ];
    for (name, workers, range_pages) in configs {
        let opts = XOptions {
            range_pages,
            ..XOptions::default()
        };
        let times = with_store(0, &f, workers, |rt, store, pool| {
            let budget = Budget::new(1 << 30);
            programs
                .iter()
                .map(|(q, p)| {
                    let mut best = std::time::Duration::MAX;
                    for _ in 0..3 {
                        let g = budget.admit(64 << 20).expect("a grant fits");
                        let start = std::time::Instant::now();
                        exec::execute(rt.clone(), p.clone(), call(store, g, pool, opts), &[])
                            .expect("a value");
                        best = best.min(start.elapsed());
                    }
                    (*q, best)
                })
                .collect::<Vec<_>>()
        });
        for (q, t) in times {
            println!("{name:32} {t:>9.2?}  {q}");
        }
    }
}
