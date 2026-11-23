//! Write states of constructors, readers first, pending drops and staged migrations over the
//! simulator.

use crate::test_world::api;
use crate::test_world::load as open;
use adtb_testkit::process::run;

use crate::RejectReason;
use crate::catalog::{Catalog, Error as CatalogError, Read, Reader};
use crate::mvcc::{Status, StatusError, Transactions, Xid};
use crate::storage::{self, Damage, Pager, Quarantine};
use crate::testing::TxnProbe;
use crate::verify::{self, Depth};
use crate::{CallOptions, Database, Error, Log};
use adtb_lang::types::RelId;
use adtb_sim::{Config, Sim, SimRuntime};
use std::collections::BTreeSet;

use crate::test_world::load::options;
use api::query;

fn opened(rt: &SimRuntime, log: &[&str]) -> Database<SimRuntime> {
    open::open_in(rt, open::FILE, &Log::new(log.iter().copied()), &options(4))
        .unwrap_or_else(|e| panic!("the database opens at {}: {e:?}", log.len()))
}

fn write(db: &Database<SimRuntime>, text: &str) {
    db.query_text(text, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{text}` fails: {e:?}"));
}

#[must_use]
fn fails(db: &Database<SimRuntime>, text: &str) -> Error {
    match db.query_text(text, &[], &CallOptions::default()) {
        Ok(rows) => panic!("`{text}` succeeds with {} results", rows.len()),
        Err(e) => e,
    }
}

const SHOP: &str = r"module shop
add module shop
add type Customer = { id: Key customers, name: Text }
add relation customers : Customer key id generate sequence
add type Payment { id: Key payments, customer: Key customers }
  = Card { last4: Text }
  | Invoice { due: Text }
add relation payments : Payment
  key id generate sequence
  index (customer)
add type Badge { name: Text } = Plain | Color { rgb: I32 }
add relation post_badges : Badge key name
add relation user_badges : Badge key name
add set Simple of post_badges = Plain
add type Provider = Local | Google
add type Account = { id: Key accounts, provider: Provider, note: Text }
add relation accounts : Account key id generate sequence
add type Hold { id: Key holds } = Live { payment: Key payments on delete retag Lapsed {} } | Lapsed
add relation holds : Hold key id generate sequence
";

const ROWS: &str = r#"transact
  let c = insert customers { name: "ann" }
  insert payments.Card { customer: key c, last4: "4242" }
  insert payments.Invoice { customer: key c, due: "2027-01" }
  insert customers { name: "bob" }
  insert post_badges.Plain { name: "new" }
  insert accounts { provider: Local, note: "a" }"#;

#[test]
fn a_crash_at_every_flush_of_a_staged_migration_reopens_to_none_or_all_of_it() {
    let seed = 26;
    let staged = r#"module shop
add unique (name) in customers
add index (last4) in payments.Card
add constraint no_blank = from c in customers where c.name == "" select c
"#;
    let many: String = (0..40)
        .map(|i| format!("  insert customers {{ name: \"c{i}\" }}\n"))
        .collect();
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    let mut start = 0;
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        write(&db, ROWS);
        write(&db, &format!("transact\n{many}"));
        db.close().expect("the database closes");
        start = rt.recorded();
        let db = opened(&rt, &[SHOP, staged]);
        db.close().expect("the database closes");
    });
    let recording = sim.recording();
    let flushes: Vec<usize> = recording
        .flushes()
        .into_iter()
        .filter(|&f| f >= start)
        .collect();
    assert!(flushes.len() > 2, "{} flushes", flushes.len());
    for at in flushes {
        let at_seed = seed + at as u64;
        let files = recording.images(at, 1, at_seed).swap_remove(0);
        let mut sim = Sim::with_files(at_seed, Config::default(), files.clone());
        let version = run(&mut sim, |rt| {
            let (pager, catalog, txns) = catalog_at(&rt);
            assert_clean(&pager, &catalog, &txns, at);
            let version = catalog.source().version();
            pager.close();
            version
        });
        assert!(
            version == 1 || version == 2,
            "a crash at event {at} reopens at version {version}"
        );
        let mut sim = Sim::with_files(at_seed, Config::default(), files);
        run(&mut sim, |rt| {
            let db = opened(&rt, &[SHOP, staged]);
            assert_eq!(
                42,
                query(&db, "from c in customers select c.name").len(),
                "after a crash at event {at}"
            );
            let got = fails(&db, r#"insert customers { name: "c1" }"#);
            assert!(
                matches!(
                    got,
                    Error::Rejected {
                        reason: RejectReason::Violated { .. } | RejectReason::KeyViolation { .. }
                    }
                ),
                "after a crash at event {at}, the unique does not hold: {got:?}"
            );
        });
    }
}

fn catalog_at(rt: &SimRuntime) -> (Pager<SimRuntime>, Catalog, Transactions<SimRuntime>) {
    let options = storage::Options {
        page_size: 8192,
        ..storage::Options::new("app.adt")
    };
    let (pager, _) = Pager::open(rt.clone(), &options).expect("the file reopens");
    let (catalog, txns) = Catalog::open(rt.clone(), &pager).expect("the catalog opens");
    (pager, catalog, txns)
}

fn assert_clean(
    pager: &Pager<SimRuntime>,
    catalog: &Catalog,
    txns: &Transactions<SimRuntime>,
    at: usize,
) {
    let clog = txns.commit_log();
    let report = verify::verify_with(
        pager,
        &catalog.structures().inventory(Some(&clog)),
        Depth::STRUCTURE,
    );
    assert!(report.complete, "event {at}: {:?}", report.findings);
    assert!(
        report.findings.is_empty(),
        "event {at}: {:?}",
        report.findings
    );
}

const AUTH: &str = r"module auth
add module auth
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text }
  | Google { gid: Text }
add relation identities : Identity key id generate sequence
add set OAuth of identities = Google
add set Mail of identities = EmailPass
add type Session = { id: Key sessions, identity: Key identities }
add relation sessions : Session key id generate sequence
add type Provider = Local | Remote
add type Account = { id: Key accounts, provider: Provider, note: Text }
add relation accounts : Account key id generate sequence
add type Profile = { id: Key profiles, backup: Option Provider, bio: Text }
add relation profiles : Profile key id generate sequence
";

const AUTH_ROWS: &str = r#"transact
  let u = insert users { name: "ann" }
  insert identities.EmailPass { user: key u, email: "ann@x" }
  insert users { name: "bob" }"#;

const ADD_APPLE: &str =
    "module auth\nadd constructor Identity.Apple { sub: Text }\n  joins identities.OAuth\n";
const ENABLE_APPLE: &str = "module auth\nenable identities.Apple\n";
const ADD_OKTA_TYPE: &str = "module auth\nadd constructor Provider.Okta\n";
const ENABLE_OKTA: &str = "module auth\nenable Provider.Okta\n";

/// Open a database at `log` holding `AUTH_ROWS`, then open its catalog and run `body` over
/// it.
fn at_catalog<T>(
    seed: u64,
    log: &[&str],
    body: impl FnOnce(&mut Catalog, &Pager<SimRuntime>, &Transactions<SimRuntime>) -> T,
) -> T {
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, log);
        write(&db, AUTH_ROWS);
        db.close().expect("the database closes");
    });
    run(&mut sim, |rt| {
        let (pager, mut catalog, txns) = catalog_at(&rt);
        let got = body(&mut catalog, &pager, &txns);
        pager.close();
        got
    })
}

fn reader(bundle: &str, version: u64, reads: Vec<Read>) -> Reader {
    Reader {
        bundle: bundle.to_string(),
        version,
        hides_new: false,
        reads,
        writes: Vec::new(),
    }
}

fn whole(relation: &str) -> Read {
    Read::Relation(relation.to_string())
}

fn follows(relation: &str, set: Option<&str>) -> Read {
    Read::Reference {
        relation: relation.to_string(),
        set: set.map(str::to_string),
    }
}

fn field(relation: &str, name: &str) -> Read {
    Read::Field {
        relation: relation.to_string(),
        field: name.to_string(),
    }
}

/// Returns the reasons a migration of `text` is refused for.
fn refused(got: Result<(), CatalogError>) -> Vec<String> {
    match got {
        Err(CatalogError::Refused(why)) => why,
        other => panic!("the migration is not refused: {other:?}"),
    }
}

const UNIQUE_NAME: &str = "module auth\nadd unique (name) in users\n";
const DROP_UNIQUE_NAME: &str = "module auth\ndrop unique (name) in users\n";
const LABELS: &str = "module auth\nadd type Label = { name: Text, lang: Text }\nadd relation labels : Label key name\n";
const REKEY_LABELS: &str = "module auth\nrekey relation labels key (name, lang)\n";

/// An object of relation `relation` that migration `added` adds and migration `dropped` drops.
struct Dropping {
    added: &'static str,
    dropped: &'static str,
    relation: &'static str,
}

const UNIQUE: Dropping = Dropping {
    added: UNIQUE_NAME,
    dropped: DROP_UNIQUE_NAME,
    relation: "users",
};

const OLD_KEY: Dropping = Dropping {
    added: LABELS,
    dropped: REKEY_LABELS,
    relation: "labels",
};

impl Dropping {
    fn whole(&self) -> Read {
        whole(&format!("auth::{}", self.relation))
    }
}

/// Returns the number of unique indexes, the key's among them, of relation `auth::name` in the
/// planner's description of `catalog`.
fn unique_indexes(catalog: &Catalog, name: &str) -> usize {
    let schema = catalog.schema();
    let at = schema
        .relations
        .iter()
        .position(|r| r.name == name && schema.module(r.module).name == "auth")
        .unwrap_or_else(|| panic!("auth declares {name}"));
    let r = RelId(u32::try_from(at).expect("a relation id fits 32 bits"));
    catalog.plan().relations()[&r]
        .indexes
        .iter()
        .filter(|i| i.unique)
        .count()
}

/// What a catalog call failing on an unreadable catalog root leaves: its error, whether the
/// database is refused whole, the transactions left running and what the commit log records of
/// the id the call began.
type Unread = (
    Result<(), CatalogError>,
    bool,
    BTreeSet<Xid>,
    Result<Status, StatusError>,
);

/// Returns what `call` leaves when the catalog root of `catalog` reads as unreadable.
fn with_unreadable_root(
    catalog: &mut Catalog,
    pager: &Pager<SimRuntime>,
    txns: &Transactions<SimRuntime>,
    call: impl FnOnce(&mut Catalog) -> Result<(), CatalogError>,
) -> Unread {
    pager.plant_damage(catalog.root(), Damage::Unreadable);
    let x = TxnProbe::of(txns).next_xid();
    let got = call(catalog);
    let marked = pager.is_quarantined(Quarantine::Database);
    (got, marked, txns.running(), txns.recorded(pager, x))
}

fn unreadable_root(catalog: &Catalog) -> Unread {
    let damage = Damage::Unreadable;
    let pid = catalog.root();
    let e = CatalogError::Page(storage::PageError { pid, damage });
    (Err(e), false, BTreeSet::new(), Ok(Status::Aborted))
}

#[test]
fn a_migration_failing_on_an_unreadable_catalog_root_leaves_no_transaction_running() {
    at_catalog(134, &[AUTH], |catalog, pager, txns| {
        let got = with_unreadable_root(catalog, pager, txns, |c| {
            c.migrate(pager, txns, UNIQUE_NAME).map(drop)
        });
        assert_eq!(unreadable_root(catalog), got);
    });
}

#[test]
fn a_release_failing_on_an_unreadable_catalog_root_leaves_no_transaction_running() {
    at_catalog(135, &[AUTH, UNIQUE_NAME], |catalog, pager, txns| {
        catalog
            .register(reader("web-2", 2, vec![UNIQUE.whole()]))
            .expect("the reader registers");
        catalog
            .migrate(pager, txns, DROP_UNIQUE_NAME)
            .expect("the drop applies");
        let got = with_unreadable_root(catalog, pager, txns, |c| c.release(pager, txns, "web-2"));
        assert_eq!(unreadable_root(catalog), got);
    });
}

/// Run `body` over the catalog of a file at `[AUTH, d.added, d.dropped]` whose drop `web-2` held
/// pending in an earlier open of the catalog.
fn reopened_with_pending<T>(
    sim: &mut Sim,
    d: &Dropping,
    body: impl FnOnce(&SimRuntime, &mut Catalog, &Pager<SimRuntime>, &Transactions<SimRuntime>) -> T,
) -> T {
    run(sim, |rt| {
        let db = opened(&rt, &[AUTH, d.added]);
        write(&db, AUTH_ROWS);
        db.close().expect("the database closes");
    });
    run(sim, |rt| {
        let (pager, mut catalog, txns) = catalog_at(&rt);
        catalog
            .register(reader("web-2", 2, vec![d.whole()]))
            .expect("the reader registers");
        catalog
            .migrate(&pager, &txns, d.dropped)
            .expect("the drop applies");
        pager.close();
    });
    run(sim, |rt| {
        let (pager, mut catalog, txns) = catalog_at(&rt);
        let got = body(&rt, &mut catalog, &pager, &txns);
        pager.close();
        got
    })
}

fn reopens_held_or_released_after_a_crash_at_every_flush(seed: u64, d: &Dropping) {
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    let mut start = 0;
    reopened_with_pending(&mut sim, d, |rt, catalog, pager, txns| {
        start = rt.recorded();
        catalog.settle(pager, txns).expect("the holds settle");
    });
    let recording = sim.recording();
    let flushes: Vec<usize> = recording
        .flushes()
        .into_iter()
        .filter(|&f| f >= start)
        .collect();
    assert!(!flushes.is_empty(), "the release flushes");
    for at in flushes {
        let at_seed = seed + at as u64;
        let files = recording.images(at, 1, at_seed).swap_remove(0);
        let mut sim = Sim::with_files(at_seed, Config::default(), files);
        run(&mut sim, |rt| {
            let (pager, catalog, txns) = catalog_at(&rt);
            assert_clean(&pager, &catalog, &txns, at);
            let held = catalog.pending_drops().len();
            assert_eq!(
                1 + held,
                unique_indexes(&catalog, d.relation),
                "after a crash at event {at}, seed {at_seed}"
            );
            pager.close();
        });
    }
}

#[test]
fn a_crash_at_every_flush_of_releasing_a_pending_unique_reopens_held_or_released() {
    reopens_held_or_released_after_a_crash_at_every_flush(129, &UNIQUE);
}

#[test]
fn a_crash_at_every_flush_of_releasing_a_pending_old_key_reopens_held_or_released() {
    reopens_held_or_released_after_a_crash_at_every_flush(135, &OLD_KEY);
}

/// Returns the bundles of `readers` that refuse `enable` after `add`, and those that
/// registration finds stale once their release lets `enable` apply.
fn blockers(seed: u64, add: &str, enable: &str, readers: &[Reader]) -> [Vec<String>; 2] {
    at_catalog(seed, &[AUTH, add], |catalog, pager, txns| {
        for r in readers {
            catalog.register(r.clone()).expect("the reader registers");
        }
        let why = match catalog.migrate(pager, txns, enable) {
            Ok(()) => Vec::new(),
            Err(e) => refused(Err(e)),
        };
        let migration = readers
            .iter()
            .filter(|r| {
                why.iter()
                    .any(|w| w.contains(&format!("bundle {},", r.bundle)))
            })
            .map(|r| r.bundle.clone())
            .collect();
        for r in readers {
            catalog
                .release(pager, txns, &r.bundle)
                .expect("the reader is released");
        }
        catalog
            .migrate(pager, txns, enable)
            .expect("the enable applies once every reader is released");
        let registration = readers
            .iter()
            .filter(|r| {
                crate::catalog::stale(catalog.source(), catalog.schema(), r)
                    .expect("the reader resolves")
                    .is_some()
            })
            .map(|r| r.bundle.clone())
            .collect();
        [migration, registration]
    })
}

#[test]
fn registration_finds_stale_exactly_the_readers_that_refuse_the_enable() {
    let hiding = |r: Reader| Reader {
        hides_new: true,
        ..r
    };
    let ctor = [
        reader("users", 1, vec![whole("auth::users")]),
        reader("whole", 1, vec![whole("auth::identities")]),
        reader("pinned-after", 2, vec![whole("auth::identities")]),
        reader("any", 1, vec![follows("auth::identities", None)]),
        reader("oauth", 1, vec![follows("auth::identities", Some("OAuth"))]),
        reader("mail", 1, vec![follows("auth::identities", Some("Mail"))]),
        reader("fields", 1, vec![field("auth::identities", "user")]),
        hiding(reader("hiding", 1, vec![whole("auth::identities")])),
    ];
    let value = [
        reader("provider", 1, vec![field("auth::accounts", "provider")]),
        reader("option", 1, vec![field("auth::profiles", "backup")]),
        hiding(reader(
            "hiding",
            1,
            vec![field("auth::accounts", "provider")],
        )),
        reader("notes", 1, vec![field("auth::accounts", "note")]),
        reader("ranges", 1, vec![whole("auth::accounts")]),
        reader("pinned-after", 2, vec![whole("auth::accounts")]),
    ];
    let cases = [
        (
            90,
            ADD_APPLE,
            ENABLE_APPLE,
            &ctor[..],
            ["whole", "any", "oauth"],
        ),
        (
            91,
            ADD_OKTA_TYPE,
            ENABLE_OKTA,
            &value[..],
            ["provider", "option", "hiding"],
        ),
    ];
    for (seed, add, enable, readers, blocking) in cases {
        let [migration, registration] = blockers(seed, add, enable, readers);
        assert_eq!(blocking.to_vec(), migration, "seed {seed}: migration");
        assert_eq!(migration, registration, "seed {seed}: registration");
    }
}
