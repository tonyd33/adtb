//! A role's binding of a bundle and its registered calls: call grants, ownership, the checks at
//! submit and on leaving the queue, definer rights, audit rows and lease renewal.

use adtb_simkit::api;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Wake, Waker};
use std::time::Duration;

use adtb::batch::{self, Batch, BatchBuilder, Type};
use adtb::key::Scalar;
use adtb::runtime::{Clock, Done, Io, Op, Runtime, ThreadId, Threads};
use adtb::{
    CallDescriptor, CallKind, CallOptions, Capability, Completion, Database, Determinism, Error,
    HostFns, Log, Observation, OpenOptions, Pins, Registration, RejectReason, RoleBinding,
    RoleSession, Task, TaskOptions, TxnCallOptions, TxnOptions, Value,
};
use adtb_lang::types::{Base, Ty};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_testkit::profile::sweep;

use api::{once, options, query, sorted};

const SHOP: &str = r#"module shop
add module shop
add type Customer = { id: Key customers, name: Text }
add relation customers : Customer key id generate sequence
add type Counter = { id: Key counters, value: I64 }
add relation counters : Counter key id generate sequence
define fn names () -> Set Text = from c in customers select c.name
define fn gate (n: I64) -> I64 = host "gate" version "1.0" deterministic
define fn whose () -> Text = match caller
  | EndUser u -> u.name
  | Service s -> s.name
  | Anonymous -> "nobody"
"#;

const GRANTS: &str = "module shop
grant read customers, insert customers, read counters, update counters to ann with grant option
grant call fn names to fns
";

const LATER: &str = "module shop\nadd type Note = { id: Key notes, text: Text }\nadd relation notes : Note key id generate sequence\n";

const LOG: &[&str] = &[SHOP, GRANTS];

const APP: &str = r#"import shop
import fn names
query q () -> Set Text = from c in customers select c.name
query other () -> Set Text = from c in customers select c.name
query me () -> Text = whose()
query named (n: Text) -> Set Text = from c in customers
  where c.name == n and match caller
    | EndUser u -> c.name == u.name
    | Service s -> s.name == c.name
    | Anonymous -> false
  select c.name
query held () -> I64 = gate(1)
txn add (n: Text) -> Key customers =
  transact
    let c = insert customers { name: n }
    return key c
txn bump () -> Key customers =
  transact
    update c in counters set { c with value: c.value + 1 }
    let c = insert customers { name: "bumped" }
    return key c
"#;

/// The callables of `APP`, in ordinal order.
const APP_CALLS: &[&str] = &["names", "q", "other", "me", "named", "held", "add", "bump"];

const UPDATE: &str = "update c in counters set { c with value: c.value + 1 }";

const ONE_Q: &str = "import shop\nquery q () -> Set Text = from c in customers select c.name\n";

const BY_TEXT: &str = "import shop\nquery q (n: Text) -> Set Text = from c in customers where c.name == n select c.name\n";

const BY_INT: &str =
    "import shop\nquery q (n: I64) -> Set Text = from c in customers select c.name\n";

/// Holds every call of the host function `gate` until released.
#[derive(Default)]
struct Gate {
    open: AtomicBool,
    entered: AtomicBool,
}

impl Gate {
    fn pass(&self, rt: &SimRuntime) {
        self.entered.store(true, Ordering::Release);
        while !self.open.load(Ordering::Acquire) {
            rt.yield_now();
        }
    }

    fn wait(&self, rt: &SimRuntime) {
        while !self.entered.load(Ordering::Acquire) {
            rt.yield_now();
        }
    }
}

fn gated(rt: &SimRuntime, gate: &Arc<Gate>) -> OpenOptions {
    let mut host = HostFns::new();
    let (grt, g) = (rt.clone(), gate.clone());
    host.register(
        "gate",
        "1.0",
        Determinism::Deterministic,
        move |b: &Batch| {
            g.pass(&grt);
            let mut out = BatchBuilder::new(batch::Schema::new(vec![Type::Scalar(Scalar::I64)]));
            for i in 0..b.len() {
                out.push_row(&[b.value(0, i)]);
            }
            Ok(out.finish())
        },
    );
    OpenOptions {
        workers: Some(1),
        host_fns: host,
        ..options(2)
    }
}

struct Fixture {
    db: Database<SimRuntime>,
    rt: SimRuntime,
    gate: Arc<Gate>,
}

fn version_hash(log: &[&str]) -> String {
    let mut source = adtb_lang::evolve::Source::default();
    for text in log {
        let file = adtb_lang::parse::parse_migration(text, adtb_lang::parse::Options::default())
            .ok()
            .expect("the migration parses");
        source = source.migrate(&file, text).expect("the migration applies");
    }
    let schema = source.schema().expect("the source builds");
    adtb::catalog::version_hash(&schema, &source)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn bundle(name: &str, log: &[&str], body: &str) -> Registration {
    let text = format!(
        "bundle {name} pins {} ({}) edition \"2026-10\" lease 8d\n{body}",
        log.len(),
        version_hash(log)
    );
    Registration::bundle(text).expect("the bundle parses")
}

/// Returns the database at `log` with the roles `ann`, `web`, `fns` and `bob`.
fn at(rt: &SimRuntime, log: &[&str]) -> Fixture {
    let gate = Arc::new(Gate::default());
    let open = |log: &[&str]| {
        Database::open_in(
            rt.clone(),
            "binding.adt",
            &Log::new(log.iter().copied()),
            &gated(rt, &gate),
        )
        .expect("the database opens")
    };
    let db = open(&log[..1]);
    for role in ["ann", "web", "fns", "bob"] {
        db.role_authority().create_role(role).expect("the role");
    }
    db.close().expect("close");
    let db = open(log);
    Fixture {
        db,
        rt: rt.clone(),
        gate,
    }
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

fn sleep(rt: &SimRuntime, d: Duration) {
    let mut io = rt.io();
    let ticket = io.submit(Op::Timer { at: rt.now() + d });
    loop {
        if let Some(c) = io.poll() {
            assert_eq!(ticket, c.ticket);
            assert_eq!(Ok(Done::Fired), c.result);
            return;
        }
        rt.park();
    }
}

const DAY: Duration = Duration::from_secs(24 * 3600);

/// Returns the rows of `task` as printed values, or its failure, and how it completed.
fn finish(rt: &SimRuntime, task: &Task<SimRuntime>) -> Result<(Vec<String>, Completion), Error> {
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
                rows.extend(
                    b.rows()
                        .expect("a task of values")
                        .iter()
                        .map(|v| format!("{v:?}")),
                );
            }
            Observation::End => task.drained(),
            Observation::Done(Completion::Failed(e)) => return Err(e),
            Observation::Done(done) => return Ok((rows, done)),
        }
    }
}

impl Fixture {
    fn session(&self, role: &str) -> RoleSession<SimRuntime> {
        self.db.role_authority().session(role).expect("a session")
    }

    fn register(&self, role: &str, reg: &Registration) {
        self.db
            .role_authority()
            .register(&self.session(role), reg)
            .expect("it registers");
    }

    fn grant(&self, role: &str, text: &str) {
        self.db
            .role_authority()
            .grant(&self.session(role), text)
            .expect("it grants");
    }

    fn revoke(&self, role: &str, text: &str) {
        self.db
            .role_authority()
            .revoke(&self.session(role), text)
            .expect("it revokes");
    }

    fn bind(&self, role: &str, name: &str) -> Result<RoleBinding<SimRuntime>, Error> {
        self.db
            .role_authority()
            .bind(&self.session(role), name, &Pins::default())
    }

    fn call(
        &self,
        binding: &RoleBinding<SimRuntime>,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Vec<String>, Error> {
        let task = binding.submit(ordinal(binding, name), args, TaskOptions::default())?;
        finish(&self.rt, &task).map(|(rows, _)| rows)
    }

    fn renewed(&self) -> Vec<String> {
        query(&self.db, "from b in catalog::bundles select b.renewed")
    }

    fn denials(&self, role: &str) -> Vec<String> {
        sorted(api::result(
            &self.db,
            &format!("from a in catalog::audit.Denied where a.role == \"{role}\" select a.origin"),
        ))
    }

    fn calls(&self, role: &str) -> Vec<String> {
        sorted(api::result(
            &self.db,
            &format!(
                "from a in catalog::audit.Call where a.role == \"{role}\" select {{ origin: a.origin, outcome: a.outcome }}"
            ),
        ))
    }

    /// Hold the one worker on the gate until `release`.
    fn hold(&self) -> Task<SimRuntime> {
        let hold = self
            .session("owner")
            .submit_text(
                "from n in [1, 2] select gate(n)",
                &[],
                TaskOptions::default(),
            )
            .expect("the holding text is admitted");
        hold.supply(usize::MAX);
        self.gate.wait(&self.rt);
        hold
    }

    fn release(&self, hold: &Task<SimRuntime>) {
        self.gate.open.store(true, Ordering::Release);
        finish(&self.rt, hold).expect("the holding text ends");
    }
}

fn anonymous(db: &Database<SimRuntime>) -> Value {
    let schema = db.schema();
    let ctor = schema
        .ty(schema.principal)
        .ctor_index("Anonymous")
        .expect("the prelude declares it");
    Value::ctor(ctor, Vec::new())
}

fn write(db: &Database<SimRuntime>, text: &str) {
    db.query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` runs: {e:?}"));
}

fn descriptor<'b>(binding: &'b RoleBinding<SimRuntime>, name: &str) -> Option<&'b CallDescriptor> {
    binding.binding().calls.iter().find(|c| c.name == name)
}

fn described(binding: &RoleBinding<SimRuntime>) -> Vec<(u32, &str)> {
    let calls = &binding.binding().calls;
    calls.iter().map(|c| (c.ordinal, c.name.as_str())).collect()
}

/// Returns the ordinal of `name`: as described, or in `APP` for a callable the role was not told.
fn ordinal(binding: &RoleBinding<SimRuntime>, name: &str) -> u32 {
    descriptor(binding, name).map_or_else(
        || {
            let i = APP_CALLS
                .iter()
                .position(|c| *c == name)
                .unwrap_or_else(|| panic!("no callable {name}"));
            u32::try_from(i + 1).expect("a small ordinal")
        },
        |c| c.ordinal,
    )
}

fn reason<T: std::fmt::Debug>(got: Result<T, Error>) -> RejectReason {
    match got {
        Err(Error::Rejected { reason }) => reason,
        other => panic!("a rejection: {other:?}"),
    }
}

fn denied<T: std::fmt::Debug>(got: Result<T, Error>) -> String {
    match reason(got) {
        RejectReason::Denied { detail } => detail,
        other => panic!("`Denied`: {other:?}"),
    }
}

#[test]
fn a_query_grant_runs_that_query_and_refuses_another_with_its_audit_row() {
    once(301, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-2.4", LOG, APP));
        f.grant("owner", "grant call web-2.4::q to web");
        let web = f.bind("web", "web-2.4").expect("web binds");

        f.call(&web, "q", Vec::new()).expect("q runs");
        let calls = f.calls("web");
        assert_eq!(1, calls.len(), "{calls:?}");
        assert!(
            calls[0].contains("web-2.4") && calls[0].contains("Succeeded"),
            "{calls:?}"
        );

        denied(f.call(&web, "other", Vec::new()));
        assert_eq!(1, f.calls("web").len(), "a refusal writes no Call row");
        let other = ordinal(&web, "other");
        let rows = f.denials("web");
        assert_eq!(1, rows.len(), "{rows:?}");
        assert!(
            rows[0].contains("web-2.4") && rows[0].contains(&format!("query: {other}")),
            "{rows:?}"
        );
    });
}

#[test]
fn a_family_grant_runs_on_equal_ids_and_is_refused_on_differing_ones() {
    once(302, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, BY_TEXT));
        f.grant("owner", "grant call web::q to web");
        f.register("owner", &bundle("web-1.1", LOG, BY_TEXT));
        f.register("owner", &bundle("web-2.0", LOG, BY_INT));

        let same = f.bind("web", "web-1.1").expect("equal ids bind");
        f.call(&same, "q", vec![Value::text("x")])
            .expect("equal ids run");
        denied(f.bind("web", "web-2.0"));
        assert_eq!(1, f.denials("web").len());
    });
}

#[test]
fn a_family_grant_revoked_after_the_bind_refuses_the_call() {
    once(303, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, BY_TEXT));
        f.grant("owner", "grant call web::q to web");
        let web = f.bind("web", "web-1.0").expect("binds");
        f.revoke("owner", "revoke call web::q from web");
        denied(f.call(&web, "q", vec![Value::text("x")]));
    });
}

#[test]
fn a_bundle_grant_runs_every_query_and_an_imported_function_needs_its_row() {
    once(304, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, APP));
        f.grant("owner", "grant call web-1.0 to web");
        let web = f.bind("web", "web-1.0").expect("web binds");
        f.call(&web, "q", Vec::new()).expect("q");
        f.call(&web, "other", Vec::new()).expect("other");
        f.call(&web, "add", vec![Value::text("a")]).expect("add");
        denied(f.call(&web, "names", Vec::new()));
        let rows = f.denials("web");
        assert!(
            rows.len() == 1 && rows[0].contains("Function") && rows[0].contains("shop::names"),
            "{rows:?}"
        );

        let fns = f
            .bind("fns", "web-1.0")
            .expect("an imported function binds");
        assert_eq!(
            CallKind::Function,
            descriptor(&fns, "names").expect("described").kind
        );
        let got = f.call(&fns, "names", Vec::new()).expect("names runs");
        assert_eq!(1, got.len(), "{got:?}");
        denied(f.call(&fns, "q", Vec::new()));
    });
}

#[test]
fn a_bind_with_no_grant_is_denied_alike_for_a_present_and_an_absent_bundle() {
    once(305, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, ONE_Q));
        sleep(&rt, 2 * DAY);
        let before = f.renewed();

        let present = denied(f.bind("web", "web-1.0"));
        let absent = denied(f.bind("web", "web-9.9"));
        assert_eq!(
            present.replace("web-1.0", "B"),
            absent.replace("web-9.9", "B")
        );
        assert_eq!(before, f.renewed(), "a denied bind renews no lease");
        let rows = f.denials("web");
        assert_eq!(2, rows.len(), "{rows:?}");
        assert!(
            rows.iter()
                .any(|r| r.contains("web-1.0") && r.contains("query: 0"))
        );
        assert!(
            rows.iter()
                .any(|r| r.contains("web-9.9") && r.contains("query: 0"))
        );

        f.db.role_authority()
            .grant_capability("web", Capability::Adhoc, false)
            .expect("grant");
        denied(f.bind("web", "web-1.0"));
        assert_eq!(before, f.renewed(), "`adhoc` alone does not bind");

        f.grant("owner", "grant call web-1.0::q to web");
        f.bind("web", "web-1.0").expect("a holder binds");
        assert_ne!(before, f.renewed(), "a successful bind renews");
    });
}

#[test]
fn a_call_for_a_role_with_a_grant_renews_only_when_it_is_served() {
    once(306, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, APP));
        f.grant("owner", "grant call web-1.0::q to web");
        let web = f.bind("web", "web-1.0").expect("binds");
        sleep(&rt, 2 * DAY);
        let before = f.renewed();
        denied(f.call(&web, "other", Vec::new()));
        assert_eq!(before, f.renewed(), "a denied call renews no lease");
        let bad = web.submit(
            ordinal(&web, "q"),
            vec![Value::Int(1)],
            TaskOptions::default(),
        );
        assert!(
            matches!(reason(bad.map(drop)), RejectReason::Invalid { .. }),
            "too many arguments"
        );
        assert_eq!(before, f.renewed(), "a rejected argument renews no lease");
        assert!(
            f.calls("web").is_empty(),
            "invalid arguments write no Call row"
        );
        f.call(&web, "q", Vec::new()).expect("q");
        assert_ne!(before, f.renewed(), "a served call renews");
    });
}

#[test]
fn the_owners_run_with_no_grant_and_a_family_owner_only_on_bundles_it_owns() {
    once(307, |rt| {
        let f = at(&rt, LOG);
        f.register("ann", &bundle("web-1.0", LOG, ONE_Q));
        f.register("owner", &bundle("web-2.0", LOG, ONE_Q));

        let own = f.bind("ann", "web-1.0").expect("the bundle's owner binds");
        f.call(&own, "q", Vec::new()).expect("with no grant");
        denied(f.bind("ann", "web-2.0"));
        for name in ["web-1.0", "web-2.0"] {
            let schema = f.bind("owner", name).expect("the schema owner binds");
            f.call(&schema, "q", Vec::new()).expect("with no grant");
        }
        assert!(matches!(
            reason(f.bind("owner", "web-9.9").map(drop)),
            RejectReason::UnknownBundle { .. }
        ));
    });
}

#[test]
fn a_revoke_while_queued_refuses_with_no_renewal() {
    once(308, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, APP));
        f.grant("owner", "grant call web-1.0::q to web");
        let web = f.bind("web", "web-1.0").expect("binds");
        sleep(&rt, 2 * DAY);
        let before = f.renewed();
        let hold = f.hold();
        let task = web
            .submit(ordinal(&web, "q"), Vec::new(), TaskOptions::default())
            .expect("admitted");
        f.revoke("owner", "revoke call web-1.0::q from web");
        f.release(&hold);
        denied(finish(&rt, &task));
        assert_eq!(before, f.renewed());
        assert_eq!(1, f.denials("web").len());
        assert!(f.calls("web").is_empty());
    });
}

#[test]
fn a_grant_kept_while_queued_runs_and_renews() {
    once(309, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, APP));
        f.grant("owner", "grant call web-1.0::q to web");
        let web = f.bind("web", "web-1.0").expect("binds");
        sleep(&rt, 2 * DAY);
        let before = f.renewed();
        let hold = f.hold();
        let task = web
            .submit(ordinal(&web, "q"), Vec::new(), TaskOptions::default())
            .expect("admitted");
        f.release(&hold);
        finish(&rt, &task).expect("it runs");
        assert_ne!(before, f.renewed());
    });
}

#[test]
fn a_revoke_after_start_lets_the_call_finish_and_the_next_is_refused() {
    once(310, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, APP));
        f.grant("owner", "grant call web-1.0 to web");
        let web = f.bind("web", "web-1.0").expect("binds");
        let task = web
            .submit(ordinal(&web, "held"), Vec::new(), TaskOptions::default())
            .expect("admitted");
        task.supply(usize::MAX);
        f.gate.wait(&rt);
        f.revoke("owner", "revoke call web-1.0 from web");
        f.gate.open.store(true, Ordering::Release);
        finish(&rt, &task).expect("a started call finishes");
        assert!(f.denials("web").is_empty());
        denied(f.call(&web, "q", Vec::new()));
    });
}

#[test]
fn a_retried_transaction_is_not_checked_again() {
    once(311, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, APP));
        f.grant("owner", "grant call web-1.0 to web");
        write(&f.db, "insert counters { value: 0 }");
        let web = f.bind("web", "web-1.0").expect("binds");
        let mut txn = f.db.begin(&TxnOptions::default()).expect("begins");
        txn.query_text(UPDATE, &[], &TxnCallOptions::default())
            .expect("updates");
        let task = web
            .submit(ordinal(&web, "bump"), Vec::new(), TaskOptions::default())
            .expect("admitted");
        task.supply(usize::MAX);
        sleep(&rt, Duration::from_millis(10));
        f.revoke("owner", "revoke call web-1.0 from web");
        txn.commit(None).expect("commits");
        let (_, done) = finish(&rt, &task).expect("the retry commits");
        let Completion::Committed { attempts, .. } = done else {
            panic!("a commit: {done:?}");
        };
        assert_eq!(2, attempts);
        assert!(f.denials("web").is_empty());
        let calls = f.calls("web");
        assert_eq!(1, calls.len(), "{calls:?}");
        assert!(calls[0].contains("Succeeded"), "{calls:?}");
    });
}

#[test]
fn retired_and_superseded_bundles_refuse_binds_and_calls_with_a_call_row() {
    once(312, |rt| {
        let f = at(&rt, LOG);
        for name in ["web-1.0", "web-2.0", "web-3.0"] {
            f.register("owner", &bundle(name, LOG, ONE_Q));
            f.grant("owner", &format!("grant call {name} to web"));
        }
        let one = f.bind("web", "web-1.0").expect("binds");
        let two = f.bind("web", "web-2.0").expect("binds");
        query(&f.db, "retire bundle web-1.0");
        query(&f.db, "supersede bundle web-2.0 by web-3.0");

        assert!(matches!(
            reason(f.bind("web", "web-1.0").map(drop)),
            RejectReason::Retired { .. }
        ));
        assert!(matches!(
            reason(f.bind("web", "web-2.0").map(drop)),
            RejectReason::BundleSuperseded { .. }
        ));
        assert!(matches!(
            reason(f.call(&one, "q", Vec::new())),
            RejectReason::Retired { .. }
        ));
        assert!(matches!(
            reason(f.call(&two, "q", Vec::new())),
            RejectReason::BundleSuperseded { .. }
        ));
        let calls = f.calls("web");
        assert_eq!(2, calls.len(), "{calls:?}");
        assert!(calls.iter().all(|c| c.contains("Failed")), "{calls:?}");
        assert!(f.denials("web").is_empty());
    });
}

#[test]
fn a_bundle_retired_while_queued_refuses_the_call() {
    once(313, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, APP));
        f.grant("owner", "grant call web-1.0 to web");
        let web = f.bind("web", "web-1.0").expect("binds");
        let hold = f.hold();
        let task = web
            .submit(ordinal(&web, "q"), Vec::new(), TaskOptions::default())
            .expect("admitted");
        f.db.query_text("retire bundle web-1.0", &[], &CallOptions::default())
            .expect("retires");
        f.release(&hold);
        assert!(matches!(
            reason(finish(&rt, &task)),
            RejectReason::Retired { .. }
        ));
        let calls = f.calls("web");
        assert_eq!(1, calls.len(), "{calls:?}");
        assert!(calls[0].contains("Failed"), "{calls:?}");
        assert!(f.denials("web").is_empty());
    });
}

#[test]
fn an_assert_revoked_while_queued_refuses_with_no_call_row() {
    once(322, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, APP));
        f.grant("owner", "grant call web-1.0 to web");
        let authority = f.db.role_authority();
        authority
            .grant_capability("web", Capability::Assert, false)
            .expect("grant");
        let web = f.bind("web", "web-1.0").expect("binds");
        let hold = f.hold();
        let opts = TaskOptions {
            call: CallOptions {
                assert: Some(anonymous(&f.db)),
                ..CallOptions::default()
            },
            ..TaskOptions::default()
        };
        let task = web
            .submit(ordinal(&web, "me"), Vec::new(), opts)
            .expect("admitted");
        authority
            .revoke_capability("web", Capability::Assert)
            .expect("revoke");
        f.release(&hold);
        denied(finish(&rt, &task));
        assert_eq!(1, f.denials("web").len());
        assert!(f.calls("web").is_empty());
    });
}

#[test]
fn an_expired_bundle_refuses_holders_as_expired_and_others_as_denied() {
    once(323, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, ONE_Q));
        f.grant("owner", "grant call web-1.0 to web");
        let web = f.bind("web", "web-1.0").expect("binds");
        sleep(&rt, 9 * DAY);
        assert!(matches!(
            reason(f.bind("web", "web-1.0").map(drop)),
            RejectReason::BundleExpired { .. }
        ));
        assert!(matches!(
            reason(f.call(&web, "q", Vec::new())),
            RejectReason::BundleExpired { .. }
        ));
        denied(f.bind("bob", "web-1.0"));
    });
}

#[test]
fn a_dropped_roles_binding_and_session_refuse() {
    once(314, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, ONE_Q));
        f.grant("owner", "grant call web-1.0 to bob");
        let bob = f.session("bob");
        let bound = f.bind("bob", "web-1.0").expect("binds");
        f.revoke("owner", "revoke call web-1.0 from bob");
        f.db.role_authority().drop_role("bob").expect("drops");
        denied(
            f.db.role_authority()
                .bind(&bob, "web-1.0", &Pins::default()),
        );
        denied(f.call(&bound, "q", Vec::new()));
        assert_eq!(2, f.denials("bob").len());
    });
}

#[test]
fn a_session_of_a_recreated_role_is_refused_and_a_fresh_one_binds() {
    once(324, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, ONE_Q));
        f.grant("owner", "grant call web-1.0 to bob");
        let bob = f.session("bob");
        let bound = f.bind("bob", "web-1.0").expect("binds");
        f.db.role_authority().drop_role("bob").expect("drops");
        f.db.role_authority().create_role("bob").expect("recreates");
        f.grant("owner", "grant call web-1.0 to bob");
        denied(
            f.db.role_authority()
                .bind(&bob, "web-1.0", &Pins::default()),
        );
        denied(f.call(&bound, "q", Vec::new()));
        let fresh = f.bind("bob", "web-1.0").expect("a fresh session binds");
        f.call(&fresh, "q", Vec::new()).expect("and calls");
    });
}

#[test]
fn another_databases_owner_session_does_not_bind() {
    once(325, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, ONE_Q));
        let other = Database::open_in(rt, "other.adt", &Log::new([SHOP]), &options(1))
            .expect("the database opens");
        let foreign = other.role_authority().session("owner").expect("a session");
        denied(
            f.db.role_authority()
                .bind(&foreign, "web-1.0", &Pins::default()),
        );
        other.close().expect("close");
    });
}

#[test]
fn a_binding_describes_only_what_the_role_may_call_when_it_binds() {
    once(327, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, APP));
        f.grant("owner", "grant call web-1.0::other to web");
        let web = f.bind("web", "web-1.0").expect("web binds");
        assert_eq!(vec![(3, "other")], described(&web));
        let fns = f.bind("fns", "web-1.0").expect("fns binds");
        assert_eq!(vec![(1, "names")], described(&fns));
        let owner = f.bind("owner", "web-1.0").expect("the owner binds");
        let all: Vec<_> = (1..).zip(APP_CALLS.iter().copied()).collect();
        assert_eq!(all, described(&owner));

        f.grant("owner", "grant call web-1.0::q to web");
        assert_eq!(vec![(3, "other")], described(&web));
        f.call(&web, "q", Vec::new())
            .expect("a later grant calls by ordinal with no rebind");
        let again = f.bind("web", "web-1.0").expect("rebinds");
        assert_eq!(vec![(2, "q"), (3, "other")], described(&again));
    });
}

#[test]
fn a_binding_describes_a_family_grant_only_on_equal_ids() {
    once(328, |rt| {
        let f = at(&rt, LOG);
        let text = format!("{BY_TEXT}query other () -> I64 = 1\n");
        let int = format!("{BY_INT}query other () -> I64 = 1\n");
        f.register("owner", &bundle("web-1.0", LOG, &text));
        f.grant("owner", "grant call web::q to web");
        f.grant("owner", "grant call web::other to web");
        f.register("owner", &bundle("web-1.1", LOG, &text));
        f.register("owner", &bundle("web-2.0", LOG, &int));
        let same = f.bind("web", "web-1.1").expect("equal ids bind");
        assert_eq!(vec![(1, "q"), (2, "other")], described(&same));
        let differs = f.bind("web", "web-2.0").expect("other's ids are equal");
        assert_eq!(vec![(2, "other")], described(&differs));
    });
}

#[test]
fn every_refused_call_of_a_binding_has_one_detail_and_names_no_callable() {
    once(329, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, APP));
        f.grant("owner", "grant call web-1.0::q to web");
        let web = f.bind("web", "web-1.0").expect("binds");
        let no_query = denied(f.call(&web, "other", Vec::new()));
        let no_function = denied(f.call(&web, "names", Vec::new()));
        let asserted = denied(
            web.submit(
                ordinal(&web, "q"),
                Vec::new(),
                TaskOptions {
                    call: CallOptions {
                        assert: Some(anonymous(&f.db)),
                        ..CallOptions::default()
                    },
                    ..TaskOptions::default()
                },
            )
            .map(drop),
        );
        f.revoke("owner", "revoke call web-1.0::q from web");
        let revoked = denied(f.call(&web, "q", Vec::new()));
        f.db.role_authority().drop_role("web").expect("drops");
        let stale = denied(f.call(&web, "q", Vec::new()));
        for got in [&no_function, &asserted, &revoked, &stale] {
            assert_eq!(&no_query, got);
        }
        for name in APP_CALLS {
            assert!(!no_query.contains(name), "{no_query}");
        }
        assert!(!no_query.contains("web-1.0"), "{no_query}");
    });
}

#[test]
fn an_unknown_ordinal_is_answered_before_authority() {
    once(339, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, APP));
        f.grant("owner", "grant call web-1.0::q to web");
        let web = f.bind("web", "web-1.0").expect("binds");
        let past = u32::try_from(APP_CALLS.len() + 1).expect("a small count");
        for ordinal in [0, past, u32::MAX] {
            assert!(matches!(
                reason(
                    web.submit(ordinal, Vec::new(), TaskOptions::default())
                        .map(drop)
                ),
                RejectReason::UnknownQuery
            ));
        }
        f.db.role_authority().drop_role("web").expect("drops");
        assert!(matches!(
            reason(
                web.submit(past, Vec::new(), TaskOptions::default())
                    .map(drop)
            ),
            RejectReason::UnknownQuery
        ));
        assert!(f.denials("web").is_empty());
    });
}

#[test]
fn an_owners_queued_call_is_readmitted() {
    once(340, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, ONE_Q));
        let owner = f.bind("owner", "web-1.0").expect("binds");
        f.call(&owner, "q", Vec::new()).expect("the owner calls");
        let held = f.hold();
        let task = owner
            .submit(1, Vec::new(), TaskOptions::default())
            .expect("admitted");
        f.release(&held);
        finish(&rt, &task).expect("the owner's queued call is readmitted");
    });
}

#[test]
fn a_call_runs_with_definer_rights_for_the_callers_own_principal() {
    once(315, |rt| {
        let f = at(&rt, LOG);
        f.register("ann", &bundle("web-1.0", LOG, APP));
        f.grant("ann", "grant call web-1.0 to web");
        write(
            &f.db,
            "insert customers { name: \"web\" }\ninsert customers { name: \"ann\" }",
        );
        let web = f.bind("web", "web-1.0").expect("binds");

        let all = f
            .call(&web, "q", Vec::new())
            .expect("reads with no relation grant");
        assert_eq!(2, all.len(), "{all:?}");
        f.call(&web, "add", vec![Value::text("new")])
            .expect("writes with no relation grant");
        assert_eq!(
            1,
            query(
                &f.db,
                "from c in customers where c.name == \"new\" select c.name"
            )
            .len()
        );
        let me = f.call(&web, "me", Vec::new()).expect("me");
        assert!(me[0].contains("web") && !me[0].contains("ann"), "{me:?}");
        assert_eq!(
            1,
            f.call(&web, "named", vec![Value::text("web")])
                .expect("own")
                .len()
        );
        assert!(
            f.call(&web, "named", vec![Value::text("ann")])
                .expect("a parameter chooses no principal")
                .is_empty()
        );
    });
}

#[test]
fn an_assertion_needs_the_capability_and_a_refusal_is_audited() {
    once(316, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, APP));
        f.grant("owner", "grant call web-1.0 to web");
        let web = f.bind("web", "web-1.0").expect("binds");
        let asserted = |assert: Value| TaskOptions {
            call: CallOptions {
                assert: Some(assert),
                ..CallOptions::default()
            },
            ..TaskOptions::default()
        };
        let principal = anonymous(&f.db);
        denied(
            web.submit(ordinal(&web, "me"), Vec::new(), asserted(principal.clone()))
                .map(drop),
        );
        assert_eq!(1, f.denials("web").len());
        f.db.role_authority()
            .grant_capability("web", Capability::Assert, false)
            .expect("grant");
        let task = web
            .submit(ordinal(&web, "me"), Vec::new(), asserted(principal))
            .expect("admitted");
        let (rows, _) = finish(&rt, &task).expect("runs");
        assert!(rows[0].contains("nobody"), "{rows:?}");
    });
}

#[test]
fn a_call_at_an_older_pin_compares_the_grant_at_that_pin() {
    once(317, |rt| {
        let log: &[&str] = &[
            SHOP,
            "module shop\nadd type Pick = A | B\n",
            "module shop\nadd constructor Pick.C\n",
        ];
        let f = at(&rt, log);
        let body =
            "import shop\nquery q (p: Pick) -> Set Text = from c in customers select c.name\n";
        f.register("owner", &bundle("web-1.0", &log[..2], body));
        f.grant("owner", "grant call web::q to web");
        f.register("owner", &bundle("web-2.0", log, body));
        let old = f.bind("web", "web-1.0").expect("equal ids at the pin bind");
        assert_eq!(Some(2), old.binding().pins);
        denied(f.bind("web", "web-2.0"));
    });
}

#[test]
fn a_bind_compares_the_pins_only_for_a_role_that_holds_a_call() {
    once(318, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, ONE_Q));
        f.grant("owner", "grant call web-1.0 to web");
        sleep(&rt, 2 * DAY);
        let before = f.renewed();
        let authority = f.db.role_authority();
        let bind =
            |role: &str, pins: Pins| authority.bind(&f.session(role), "web-1.0", &pins).map(drop);
        let hash = version_hash(LOG);
        assert!(matches!(
            reason(bind(
                "web",
                Pins {
                    version: Some((1, hash.clone())),
                    catalog: None
                }
            )),
            RejectReason::VersionMismatch { version: 2, .. }
        ));
        assert!(matches!(
            reason(bind(
                "web",
                Pins {
                    version: Some((2, "0".repeat(64))),
                    catalog: None
                }
            )),
            RejectReason::VersionMismatch { .. }
        ));
        assert!(matches!(
            reason(bind(
                "web",
                Pins {
                    version: None,
                    catalog: Some(4)
                }
            )),
            RejectReason::CatalogMismatch { .. }
        ));
        denied(bind(
            "bob",
            Pins {
                version: Some((1, hash.clone())),
                catalog: None,
            },
        ));
        assert_eq!(before, f.renewed());
        bind(
            "web",
            Pins {
                version: Some((2, hash)),
                catalog: None,
            },
        )
        .expect("the pins agree");
        assert_ne!(before, f.renewed());
    });
}

#[test]
fn the_descriptors_are_those_of_the_pin() {
    once(319, |rt| {
        let f = at(&rt, &[SHOP, GRANTS, LATER]);
        f.register("owner", &bundle("web-1.0", LOG, APP));
        let b = f.bind("owner", "web-1.0").expect("binds");
        let got = b.binding();
        assert_eq!("web-1.0", got.bundle);
        assert_eq!("2026-10", got.edition);
        assert_eq!(Some(2), got.pins);
        assert!(got.lease_until.is_some());
        let add = descriptor(&b, "add").expect("described");
        assert_eq!(CallKind::Transaction, add.kind);
        assert_eq!("n", add.params[0].0);
        assert_eq!(Ty::Base(Base::Text), add.params[0].1);
    });
}

#[test]
fn a_whole_bundle_option_of_other_ids_closes_no_cycle_with_a_family_row() {
    once(320, |rt| {
        let f = at(&rt, LOG);
        f.register("ann", &bundle("web-1.0", LOG, BY_TEXT));
        f.grant("ann", "grant call web::q to bob with grant option");
        f.register("ann", &bundle("web-2.0", LOG, BY_INT));
        f.register("ann", &bundle("web-3.0", LOG, BY_TEXT));
        f.grant("owner", "grant call web-2.0 to bob with grant option");
        f.grant("owner", "grant call web-3.0 to bob with grant option");

        f.grant("bob", "grant call web-2.0 to ann with grant option");
        let same = f.db.role_authority().grant(
            &f.session("bob"),
            "grant call web-3.0 to ann with grant option",
        );
        assert!(
            matches!(reason(same), RejectReason::Invalid { detail } if detail.contains("cycle"))
        );
    });
}

#[test]
#[ignore = "a `Commit` row states the commit position inside that commit, which the engine learns only from the commit record, and a change set in a wire encoding the engine lacks"]
fn a_registered_commit_writes_its_commit_row() {
    once(321, |rt| {
        let f = at(&rt, LOG);
        f.register("owner", &bundle("web-1.0", LOG, APP));
        f.grant("owner", "grant call web-1.0 to web");
        let web = f.bind("web", "web-1.0").expect("binds");
        f.call(&web, "add", vec![Value::text("a")]).expect("add");
        let rows = query(
            &f.db,
            "from a in catalog::audit.Commit where a.role == \"web\" select a.origin",
        );
        assert_eq!(1, rows.len(), "{rows:?}");
    });
}

#[test]
fn concurrent_bind_revoke_and_retire_keep_the_audit_and_renewal_facts() {
    for seed in sweep(330..334) {
        let mut sim = Sim::new(
            seed,
            Config {
                switch_rate: 0.3,
                ..Config::default()
            },
        );
        let got = sim.run(|rt| {
            let f = at(&rt, LOG);
            f.register("owner", &bundle("web-1.0", LOG, ONE_Q));
            f.grant("owner", "grant call web-1.0 to web");
            sleep(&rt, 2 * DAY);
            let before = f.renewed();
            let outcomes = Arc::new(Mutex::new(Vec::new()));
            let session = f.session("web");
            let threads: Vec<_> = (0..3)
                .map(|actor| {
                    let (db, session) = (f.db.clone(), session.clone());
                    let outcomes = outcomes.clone();
                    rt.spawn("actor", move || {
                        let authority = db.role_authority();
                        let owner = authority.session("owner").expect("a session");
                        match actor {
                            0 => authority
                                .revoke(&owner, "revoke call web-1.0 from web")
                                .expect("revokes"),
                            1 => {
                                db.query_text(
                                    "retire bundle web-1.0",
                                    &[],
                                    &CallOptions::default(),
                                )
                                .expect("retires");
                            }
                            _ => outcomes.lock().unwrap().push(
                                authority
                                    .bind(&session, "web-1.0", &Pins::default())
                                    .map(drop),
                            ),
                        }
                    })
                })
                .collect();
            for t in threads {
                rt.join(t);
            }
            let outcomes = outcomes.lock().unwrap();
            let refused_denied = outcomes
                .iter()
                .filter(|o| {
                    matches!(
                        o,
                        Err(Error::Rejected {
                            reason: RejectReason::Denied { .. }
                        })
                    )
                })
                .count();
            assert_eq!(
                refused_denied,
                f.denials("web").len(),
                "seed={seed}: {outcomes:?}"
            );
            if outcomes.iter().all(Result::is_err) {
                assert_eq!(before, f.renewed(), "seed={seed}: {outcomes:?}");
            }
            denied(f.bind("web", "web-1.0"));
        });
        assert_eq!(Outcome::Finished, got, "seed={seed}");
    }
}

const FILES: &str = "module shop
add type Named = { name: Text }
add relation loaded : Named key name
grant insert loaded, read loaded to ann with grant option
";

const SYSTEM_LOG: &[&str] = &[SHOP, GRANTS, FILES];

const SCAN: &str = r#"import shop
txn load () -> I64 =
  transact
    insert loaded from (from r in read_csv("names.csv", Named) select r)
    return 1
"#;

const CTORS: &str = "query ctors () -> Set Text = from c in catalog::constructors where c.name == \"Customer\" select c.name\n";

const CTOR_SUMS: &str =
    "query sums () -> Set Key catalog::types = from c in catalog::constructors select c.sum\n";

const READ_CTORS: &str = "grant read catalog::constructors to ann with grant option";

const REVOKE_CTORS: &str = "revoke read catalog::constructors from ann";

fn refused_register(f: &Fixture, role: &str, reg: &Registration) -> String {
    match f.db.role_authority().register(&f.session(role), reg) {
        Err(Error::Rejected {
            reason: RejectReason::Denied { detail },
        }) => detail,
        other => panic!("not denied: {other:?}"),
    }
}

fn revoke_refusal(f: &Fixture, role: &str, text: &str) -> String {
    match f.db.role_authority().revoke(&f.session(role), text) {
        Err(Error::Rejected {
            reason: RejectReason::Invalid { detail },
        }) => detail,
        other => panic!("not refused: {other:?}"),
    }
}

fn grant_rows(f: &Fixture, grantee: &str) -> Vec<String> {
    sorted(api::result(
        &f.db,
        &format!(
            "from g in catalog::grants where g.grantee == \"{grantee}\" select {{ privilege: g.privilege, target: g.target, option: g.grant_option }}"
        ),
    ))
}

#[test]
fn a_bundle_owner_needs_read_files_to_register_a_file_scan() {
    once(340, |rt| {
        let f = at(&rt, SYSTEM_LOG);
        let scan = bundle("load-1.0", SYSTEM_LOG, SCAN);
        assert_eq!(
            "load reads files; ann lacks read_files",
            refused_register(&f, "ann", &scan)
        );
        f.grant("owner", "grant read_files to bob");
        assert!(
            refused_register(&f, "bob", &scan).contains("bob lacks insert with grant option"),
            "read_files gives no relation grant"
        );
        f.grant("owner", "grant read_files to ann");
        f.register("ann", &scan);
    });
}

#[test]
fn a_bundle_reading_the_catalog_runs_for_a_caller_with_only_the_call_grant() {
    once(341, |rt| {
        let f = at(&rt, LOG);
        let app = bundle("cat-1.0", LOG, CTORS);
        let lacks = refused_register(&f, "ann", &app);
        assert!(
            lacks
                .contains("ctors reads constructors.Constructor; ann lacks read with grant option"),
            "{lacks}"
        );
        f.grant("owner", "grant read catalog::constructors to ann");
        assert!(
            refused_register(&f, "ann", &app).contains("with grant option"),
            "a grant without the option does not register"
        );
        f.revoke("owner", REVOKE_CTORS);
        f.grant("owner", READ_CTORS);
        f.register("ann", &app);
        f.grant("ann", "grant call cat-1.0 to web");
        let web = f.bind("web", "cat-1.0").expect("web binds");
        assert_eq!(
            vec!["Text(\"Customer\")".to_owned()],
            f.call(&web, "ctors", Vec::new()).expect("the call runs")
        );
    });
}

#[test]
fn a_role_without_schema_gets_one_denial_giving_a_system_relation_grant() {
    once(342, |rt| {
        let f = at(&rt, LOG);
        let authority = f.db.role_authority();
        let session = f.session("bob");
        let denial = |text: &str| denied(authority.grant(&session, text));
        let present = denial("grant read catalog::constructors to ann");
        assert_eq!(present, denial("grant read catalog::nosuch to ann"));
        assert_eq!(present, denial("grant read nosuch.Nope (x) to ann"));
        assert_eq!(present, denial("grant read catalog::nosuch to nobody"));
        assert!(
            grant_rows(&f, "ann")
                .iter()
                .all(|r| !r.contains("catalog::"))
        );

        f.grant("owner", "grant schema to bob");
        f.db.role_authority()
            .grant(&f.session("bob"), READ_CTORS)
            .expect("a schema holder gives it");
        assert!(
            grant_rows(&f, "ann")
                .iter()
                .any(|r| r.contains("catalog::constructors frozen (") && r.contains("option: True")),
            "{:?}",
            grant_rows(&f, "ann")
        );
        let invalid =
            f.db.role_authority()
                .grant(&f.session("bob"), "grant read catalog::nosuch to ann");
        assert!(matches!(reason(invalid), RejectReason::Invalid { .. }));
        let operator =
            f.db.role_authority()
                .grant(&f.session("bob"), "grant read customers to ann");
        assert!(matches!(reason(operator), RejectReason::Invalid { .. }));
    });
}

#[test]
fn an_option_on_a_system_relation_gives_no_right_to_grant_it() {
    once(343, |rt| {
        let f = at(&rt, LOG);
        f.grant("owner", READ_CTORS);
        denied(
            f.db.role_authority()
                .grant(&f.session("ann"), "grant read catalog::constructors to bob"),
        );
    });
}

#[test]
fn a_system_relation_grant_closing_an_option_cycle_is_refused() {
    once(344, |rt| {
        let f = at(&rt, LOG);
        f.grant("owner", "grant schema to ann");
        f.grant("owner", "grant schema to bob");
        f.grant(
            "ann",
            "grant read catalog::constructors to bob with grant option",
        );
        let closed = f.db.role_authority().grant(
            &f.session("bob"),
            "grant read catalog::constructors to ann with grant option",
        );
        assert!(matches!(reason(closed), RejectReason::Invalid { .. }));
    });
}

#[test]
fn a_refused_system_revoke_names_each_bundle_and_passes_once_retired() {
    once(345, |rt| {
        let f = at(&rt, LOG);
        f.grant("owner", READ_CTORS);
        f.register("ann", &bundle("cat-1.0", LOG, CTORS));
        f.register("ann", &bundle("dog-1.0", LOG, CTORS));
        let why = revoke_refusal(&f, "owner", REVOKE_CTORS);
        assert!(why.contains("bundle cat-1.0: ctors reads"), "{why}");
        assert!(why.contains("bundle dog-1.0: ctors reads"), "{why}");
        query(&f.db, "retire bundle cat-1.0");
        let why = revoke_refusal(&f, "owner", REVOKE_CTORS);
        assert!(!why.contains("cat-1.0") && why.contains("dog-1.0"), "{why}");
        query(&f.db, "retire bundle dog-1.0");
        f.revoke("owner", REVOKE_CTORS);
        assert!(
            grant_rows(&f, "ann")
                .iter()
                .all(|r| !r.contains("constructors"))
        );
    });
}

#[test]
fn a_refused_read_files_revoke_names_the_bundle_and_a_superseded_one_releases_it() {
    once(346, |rt| {
        let f = at(&rt, SYSTEM_LOG);
        f.grant("owner", "grant read_files to ann");
        f.register("ann", &bundle("load-1.0", SYSTEM_LOG, SCAN));
        let why = revoke_refusal(&f, "owner", "revoke read_files from ann");
        assert!(
            why.contains("bundle load-1.0: load reads files; ann lacks read_files"),
            "{why}"
        );
        assert!(matches!(
            reason(
                f.db.role_authority()
                    .revoke_capability("ann", Capability::ReadFiles)
            ),
            RejectReason::Invalid { .. }
        ));
        f.register(
            "ann",
            &bundle(
                "load-2.0",
                SYSTEM_LOG,
                "import shop\nquery q () -> Set Text = from c in customers select c.name\n",
            ),
        );
        query(&f.db, "supersede bundle load-1.0 by load-2.0");
        f.revoke("owner", "revoke read_files from ann");
    });
}

#[test]
fn an_expired_bundle_releases_a_system_revoke() {
    once(347, |rt| {
        let f = at(&rt, LOG);
        f.grant("owner", READ_CTORS);
        f.register("ann", &bundle("cat-1.0", LOG, CTORS));
        revoke_refusal(&f, "owner", REVOKE_CTORS);
        sleep(&rt, 9 * DAY);
        f.revoke("owner", REVOKE_CTORS);
    });
}

#[test]
fn a_revoke_leaving_every_live_bundle_what_it_uses_passes() {
    once(348, |rt| {
        let f = at(&rt, SYSTEM_LOG);
        f.grant("owner", READ_CTORS);
        f.grant(
            "owner",
            "grant read catalog::relations to ann with grant option",
        );
        f.grant("owner", "grant read_files to ann");
        f.grant(
            "owner",
            "grant read catalog::constructors to bob with grant option",
        );
        f.grant("owner", "grant monitor to ann");
        f.register("ann", &bundle("cat-1.0", SYSTEM_LOG, CTORS));
        f.revoke("owner", "revoke read catalog::relations from ann");
        f.revoke("owner", "revoke read_files from ann");
        f.revoke("owner", "revoke monitor from ann");
        f.revoke("owner", "revoke read catalog::constructors from bob");
    });
}

#[test]
fn a_second_grantor_keeps_a_bundle_through_a_revoke_of_the_first() {
    once(349, |rt| {
        let f = at(&rt, LOG);
        f.grant("owner", "grant schema to bob");
        f.grant("owner", READ_CTORS);
        f.db.role_authority()
            .grant(&f.session("bob"), READ_CTORS)
            .expect("bob gives it");
        f.register("ann", &bundle("cat-1.0", LOG, CTORS));
        f.db.role_authority()
            .revoke(&f.session("bob"), REVOKE_CTORS)
            .expect("the owner's row remains");
        let why = revoke_refusal(&f, "owner", REVOKE_CTORS);
        assert!(why.contains("bundle cat-1.0"), "{why}");
    });
}

#[test]
fn removing_a_system_grant_option_is_refused_the_same() {
    once(350, |rt| {
        let f = at(&rt, LOG);
        f.grant("owner", READ_CTORS);
        f.register("ann", &bundle("cat-1.0", LOG, CTORS));
        let why = revoke_refusal(&f, "owner", REVOKE_CTORS);
        assert!(why.contains("bundle cat-1.0"), "{why}");
        assert!(
            grant_rows(&f, "ann")
                .iter()
                .any(|r| r.contains("constructors") && r.contains("option: True")),
            "the refused revoke keeps the option"
        );
        query(&f.db, "retire bundle cat-1.0");
        f.revoke("owner", REVOKE_CTORS);
        f.grant("owner", "grant read catalog::constructors to ann");
        assert!(
            grant_rows(&f, "ann")
                .iter()
                .any(|r| r.contains("constructors") && r.contains("option: False")),
            "{:?}",
            grant_rows(&f, "ann")
        );
    });
}

#[test]
fn a_revoke_looks_only_at_the_bundles_of_the_revoked_role() {
    once(351, |rt| {
        let f = at(&rt, LOG);
        f.grant("owner", READ_CTORS);
        f.grant(
            "owner",
            "grant read catalog::constructors to bob with grant option",
        );
        f.register("owner", &bundle("own-1.0", LOG, CTORS));
        f.register("ann", &bundle("cat-1.0", LOG, CTORS));
        f.revoke("owner", "revoke read catalog::constructors from bob");
    });
}

#[test]
fn field_lists_on_a_system_relation_grant_cover_the_listed_fields() {
    once(352, |rt| {
        let f = at(&rt, LOG);
        f.grant(
            "owner",
            "grant read catalog::constructors (name) to ann with grant option",
        );
        assert!(
            grant_rows(&f, "ann")
                .iter()
                .any(|r| r.contains("catalog::constructors (name)")),
            "{:?}",
            grant_rows(&f, "ann")
        );
        f.register("ann", &bundle("cat-1.0", LOG, CTORS));
        let lacks = refused_register(&f, "ann", &bundle("sum-1.0", LOG, CTOR_SUMS));
        assert!(
            lacks.contains("reads constructors.Constructor (sum)") && !lacks.contains("(name)"),
            "{lacks}"
        );
        assert!(matches!(
            reason(f.db.role_authority().grant(
                &f.session("owner"),
                "grant read catalog::constructors (nosuch) to ann"
            )),
            RejectReason::Invalid { .. }
        ));
        f.grant("owner", READ_CTORS);
        f.register("ann", &bundle("sum-1.0", LOG, CTOR_SUMS));
        let why = revoke_refusal(&f, "owner", REVOKE_CTORS);
        assert!(why.contains("bundle sum-1.0"), "{why}");
    });
}

#[test]
fn a_system_relation_grant_of_constructors_covers_only_those() {
    once(353, |rt| {
        let f = at(&rt, LOG);
        f.grant(
            "owner",
            "grant read catalog::functions.Host to ann with grant option",
        );
        f.register(
            "ann",
            &bundle(
                "hosts-1.0",
                LOG,
                "query hosts () -> Set Text = from h in catalog::functions.Host select h.symbol\n",
            ),
        );
        let lacks = refused_register(
            &f,
            "ann",
            &bundle(
                "every-1.0",
                LOG,
                "query all () -> Set Text = from h in catalog::functions select h.name\n",
            ),
        );
        assert!(lacks.contains("functions.Wasm"), "{lacks}");
    });
}

fn opened(
    rt: &SimRuntime,
    log: &[&str],
    regs: Vec<Registration>,
) -> Result<Database<SimRuntime>, adtb::OpenError> {
    Database::open_in(
        rt.clone(),
        "binding.adt",
        &Log::new(log.iter().copied()),
        &OpenOptions {
            registrations: regs,
            ..gated(rt, &Arc::new(Gate::default()))
        },
    )
}

#[test]
fn a_reopen_and_a_migration_count_deployed_grants_as_registration_does() {
    once(354, |rt| {
        let f = at(&rt, SYSTEM_LOG);
        f.grant("owner", READ_CTORS);
        f.grant("owner", "grant read_files to ann");
        let cat = bundle("cat-1.0", SYSTEM_LOG, CTORS);
        let scan = bundle("load-1.0", SYSTEM_LOG, SCAN);
        f.register("ann", &cat);
        f.register("ann", &scan);
        f.db.close().expect("close");
        let db = opened(&rt, SYSTEM_LOG, vec![cat.clone(), scan])
            .unwrap_or_else(|e| panic!("the reopen counts deployed rows: {e:?}"));
        query(&db, "retire bundle load-1.0");
        db.close().expect("close");
        let later: Vec<&str> = SYSTEM_LOG.iter().copied().chain([LATER]).collect();
        let db = opened(&rt, &later, vec![cat])
            .unwrap_or_else(|e| panic!("the migration's recheck counts deployed rows: {e:?}"));
        db.close().expect("close");
    });
}

#[test]
fn a_reopen_reports_a_bundle_whose_owner_lost_a_deployed_grant() {
    once(355, |rt| {
        let f = at(&rt, LOG);
        f.grant("owner", READ_CTORS);
        let cat = bundle("cat-1.0", LOG, CTORS);
        f.register("ann", &cat);
        sleep(&rt, 9 * DAY);
        f.revoke("owner", REVOKE_CTORS);
        f.db.close().expect("close");
        match opened(&rt, LOG, vec![cat]) {
            Err(adtb::OpenError::RegistrationRefused { bundle, reason }) => {
                assert_eq!("cat-1.0", bundle);
                let why = format!("{reason:?}");
                assert!(why.contains("ann lacks read with grant option"), "{why}");
            }
            Err(other) => panic!("not refused by the recheck: {other:?}"),
            Ok(_) => panic!("the reopen admits the bundle"),
        }
    });
}

#[test]
fn registering_and_revoking_at_once_never_leave_a_live_bundle_lacking() {
    for seed in sweep(360..366) {
        let mut sim = Sim::new(
            seed,
            Config {
                switch_rate: 0.3,
                ..Config::default()
            },
        );
        let got = sim.run(|rt| {
            let f = at(&rt, LOG);
            f.grant("owner", READ_CTORS);
            let reg = bundle("cat-1.0", LOG, CTORS);
            let threads: Vec<_> = (0..2)
                .map(|actor| {
                    let (db, reg) = (f.db.clone(), reg.clone());
                    rt.spawn("actor", move || {
                        let a = db.role_authority();
                        if actor == 0 {
                            let s = a.session("owner").expect("a session");
                            drop(a.revoke(&s, REVOKE_CTORS));
                        } else {
                            let s = a.session("ann").expect("a session");
                            drop(a.register(&s, &reg));
                        }
                    })
                })
                .collect();
            for t in threads {
                rt.join(t);
            }
            let live = query(&f.db, "from b in catalog::bundles select b.name");
            let held = grant_rows(&f, "ann")
                .iter()
                .any(|r| r.contains("constructors"));
            assert!(live.is_empty() || held, "seed={seed}: {live:?}");
        });
        assert_eq!(Outcome::Finished, got, "seed={seed}");
    }
}

fn reopened(rt: &SimRuntime, log: &[&str]) -> Fixture {
    Fixture {
        db: opened(rt, log, vec![]).unwrap_or_else(|e| panic!("the reopen succeeds: {e:?}")),
        rt: rt.clone(),
        gate: Arc::new(Gate::default()),
    }
}

fn assert_expired<T: std::fmt::Debug>(got: Result<T, Error>) {
    let why = reason(got);
    assert!(matches!(why, RejectReason::BundleExpired { .. }), "{why:?}");
}

#[test]
fn raising_the_grace_brings_back_no_expired_bundle_and_registering_runs_the_checks() {
    once(370, |rt| {
        let f = at(&rt, LOG);
        f.grant("owner", READ_CTORS);
        let cat = bundle("cat-1.0", LOG, CTORS);
        f.register("ann", &cat);
        f.grant("ann", "grant call cat-1.0 to web");
        let web = f.bind("web", "cat-1.0").expect("it binds");
        sleep(&rt, 9 * DAY);
        f.revoke("owner", REVOKE_CTORS);
        api::results(&f.db, "set setting LeaseGrace = { grace: 30d }");
        assert_expired(f.bind("web", "cat-1.0"));
        assert_expired(f.call(&web, "ctors", vec![]));
        drop(web);
        f.db.close().expect("close");

        let f = reopened(&rt, LOG);
        assert_expired(f.bind("web", "cat-1.0"));
        let lacks = refused_register(&f, "ann", &cat);
        assert!(
            lacks.contains("ann lacks read with grant option"),
            "{lacks}"
        );
        assert_expired(f.bind("web", "cat-1.0"));
        f.grant("owner", READ_CTORS);
        f.register("ann", &cat);
        let web = f
            .bind("web", "cat-1.0")
            .expect("registering again clears it");
        f.call(&web, "ctors", vec![]).expect("it runs");
        drop(web);
        f.db.close().expect("close");

        let f = reopened(&rt, LOG);
        f.bind("web", "cat-1.0").expect("the clearing is durable");
    });
}

#[test]
fn a_marked_bundle_presented_at_an_open_runs_the_checks() {
    once(371, |rt| {
        let f = at(&rt, LOG);
        f.grant("owner", READ_CTORS);
        let cat = bundle("cat-1.0", LOG, CTORS);
        f.register("ann", &cat);
        sleep(&rt, 9 * DAY);
        f.revoke("owner", REVOKE_CTORS);
        api::results(&f.db, "set setting LeaseGrace = { grace: 30d }");
        f.db.close().expect("close");
        match opened(&rt, LOG, vec![cat]) {
            Err(adtb::OpenError::RegistrationRefused { bundle, .. }) => {
                assert_eq!("cat-1.0", bundle);
            }
            Err(other) => panic!("not refused by the recheck: {other:?}"),
            Ok(_) => panic!("presenting it runs the checks"),
        }
    });
}

#[test]
fn raising_the_default_lease_brings_back_no_expired_bundle() {
    once(372, |rt| {
        let f = at(&rt, LOG);
        let text = format!(
            "bundle one-1.0 pins 2 ({}) edition \"2026-10\"\n{ONE_Q}",
            version_hash(LOG)
        );
        let one = Registration::bundle(text).expect("the bundle parses");
        f.register("ann", &one);
        f.grant("ann", "grant call one-1.0 to web");
        sleep(&rt, 31 * DAY);
        api::results(&f.db, "set setting DefaultLease = { lease: 90d }");
        assert_expired(f.bind("web", "one-1.0"));
        api::results(&f.db, "set setting DefaultLease = { lease: 120d }");
        assert_expired(f.bind("web", "one-1.0"));
        f.register("ann", &one);
        f.bind("web", "one-1.0").expect("registered again");
    });
}

#[test]
fn raising_the_grace_leaves_a_bundle_not_yet_expired_live() {
    once(373, |rt| {
        let f = at(&rt, LOG);
        f.register("ann", &bundle("one-1.0", LOG, ONE_Q));
        f.grant("ann", "grant call one-1.0 to web");
        sleep(&rt, 7 * DAY);
        api::results(&f.db, "set setting LeaseGrace = { grace: 30d }");
        sleep(&rt, 2 * DAY);
        let web = f.bind("web", "one-1.0").expect("its grace is the new one");
        f.call(&web, "q", vec![]).expect("it runs");
    });
}

#[test]
fn a_bundle_a_migration_expired_past_stays_expired_when_the_grace_rises() {
    once(374, |rt| {
        let f = at(&rt, LOG);
        let one = bundle("one-1.0", LOG, ONE_Q);
        f.register("ann", &one);
        f.grant("ann", "grant call one-1.0 to web");
        sleep(&rt, 9 * DAY);
        f.db.close().expect("close");
        let later: &[&str] = &[
            SHOP,
            GRANTS,
            "module shop\nrevoke read customers from ann\n",
        ];
        let f = reopened(&rt, later);
        api::results(&f.db, "set setting LeaseGrace = { grace: 30d }");
        assert_expired(f.bind("web", "one-1.0"));
        f.db.close().expect("close");
        let f = reopened(&rt, later);
        assert_expired(f.bind("web", "one-1.0"));
        assert!(
            f.db.role_authority()
                .register(&f.session("ann"), &one)
                .is_err(),
            "registering again runs the checks"
        );
        assert_expired(f.bind("web", "one-1.0"));
    });
}

#[test]
fn a_migration_goes_past_a_bundle_marked_before_it() {
    once(375, |rt| {
        let f = at(&rt, LOG);
        f.register("ann", &bundle("one-1.0", LOG, ONE_Q));
        sleep(&rt, 9 * DAY);
        api::results(&f.db, "set setting LeaseGrace = { grace: 30d }");
        f.db.close().expect("close");
        let later: &[&str] = &[
            SHOP,
            GRANTS,
            "module shop\nrevoke read customers from ann\n",
        ];
        reopened(&rt, later);
    });
}

#[test]
fn only_the_schema_owner_grants_on_credentials_in_every_form() {
    once(376, |rt| {
        let f = at(&rt, LOG);
        f.grant("owner", "grant schema to bob");
        let forms = [
            "grant read catalog::credentials to ann",
            "grant read catalog::credentials (stored_key) to ann",
            "grant read catalog::credentials.Credential to ann",
            "grant read catalog::credentials to ann with grant option",
            "grant insert catalog::credentials to ann",
        ];
        for form in forms {
            let why = denied(f.db.role_authority().grant(&f.session("bob"), form));
            assert!(why.contains("catalog::credentials"), "{form}: {why}");
        }
        for form in &forms[..3] {
            f.grant("owner", form);
            f.revoke(
                "owner",
                &form
                    .replace("grant read", "revoke read")
                    .replace(" to ann", " from ann"),
            );
        }
        f.grant("owner", "grant read catalog::relations to ann");
    });
}

#[test]
fn a_write_grant_on_a_system_relation_is_invalid_after_authority() {
    once(377, |rt| {
        let f = at(&rt, LOG);
        f.grant("owner", "grant schema to bob");
        for privilege in ["insert", "update", "delete"] {
            let text = format!("grant {privilege} catalog::grants to ann with grant option");
            denied(f.db.role_authority().grant(&f.session("web"), &text));
            for by in ["owner", "bob"] {
                let got = f.db.role_authority().grant(&f.session(by), &text);
                match reason(got) {
                    RejectReason::Invalid { detail } => {
                        assert!(
                            detail.contains(privilege) && detail.contains("catalog::grants"),
                            "{detail}"
                        );
                    }
                    other => panic!("not invalid: {other:?}"),
                }
            }
        }
        assert!(
            grant_rows(&f, "ann")
                .iter()
                .all(|r| !r.contains("catalog::grants")),
            "{:?}",
            grant_rows(&f, "ann")
        );
    });
}

#[test]
fn a_migration_grants_no_write_on_a_system_relation() {
    once(378, |rt| {
        let f = at(&rt, LOG);
        f.db.close().expect("close");
        for privilege in ["read", "insert"] {
            let bad = format!("module shop\ngrant {privilege} catalog::grants to ann\n");
            let log: &[&str] = &[SHOP, GRANTS, &bad];
            assert!(
                opened(&rt, log, vec![]).is_err(),
                "`{privilege}` is deployment state"
            );
        }
    });
}

#[test]
fn a_role_with_nothing_gets_one_denial_revoking_any_relation() {
    once(379, |rt| {
        let f = at(&rt, LOG);
        let session = f.session("bob");
        let authority = f.db.role_authority();
        let denial = |text: &str| denied(authority.revoke(&session, text));
        let present = denial("revoke read customers from ann");
        for text in [
            "revoke read nosuch from ann",
            "revoke read customers (name) from ann",
            "revoke read customers.Customer from ann",
            "revoke read nosuch.Nope (x) from ann",
            "revoke update customers from ann",
            "revoke update nosuch from ann",
            "revoke insert customers from ann",
            "revoke delete nosuch from ann",
            "revoke retag customers -> customers from ann",
            "revoke read catalog::constructors from ann",
            "revoke read catalog::nosuch from ann",
        ] {
            assert_eq!(present, denial(text), "{text}");
        }
        for text in [
            "grant read customers (name) to ann",
            "grant read customers.Customer to ann",
            "grant retag customers -> customers to ann",
            "grant update customers to ann",
            "grant read customers, insert customers to ann",
        ] {
            let got = denied(authority.grant(&session, text));
            assert_eq!(present, got, "{text}");
        }
    });
}

#[test]
fn a_former_schema_holder_revokes_what_it_gave() {
    once(380, |rt| {
        let f = at(&rt, LOG);
        f.grant("owner", "grant schema to bob");
        f.grant("bob", "grant read catalog::relations to web");
        f.revoke("owner", "revoke schema from bob");
        f.revoke("bob", "revoke read catalog::relations from web");
        assert!(
            grant_rows(&f, "web")
                .iter()
                .all(|r| !r.contains("catalog::relations")),
            "{:?}",
            grant_rows(&f, "web")
        );
    });
}
