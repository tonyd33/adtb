//! Write states of constructors, readers first, pending drops and staged migrations over the
//! simulator.

use crate::test_world::api;
use crate::test_world::load as open;
use adtb_harness::process::run;

use crate::RejectReason;
use crate::catalog::{Catalog, Error as CatalogError, PendingDrop, Read, Reader};
use crate::mvcc::Transactions;
use crate::storage::{self, Pager};
use crate::verify::{self, Depth};
use crate::{CallOptions, Database, Error, Log};
use adtb_lang::types::RelId;
use adtb_sim::{Config, Sim, SimRuntime};

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
            let version = catalog.source().version;
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
    let report = verify::verify_with(pager, &catalog.inventory(&clog), Depth::STRUCTURE);
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

/// Returns whether `enable` applies to a catalog at `AUTH` and `add`, with `readers` live.
fn enables(seed: u64, add: &str, enable: &str, readers: Vec<Reader>) -> Result<(), CatalogError> {
    at_catalog(seed, &[AUTH, add], |catalog, pager, txns| {
        for r in readers {
            catalog.register(r).expect("the reader registers");
        }
        catalog.migrate(pager, txns, enable)
    })
}

#[test]
fn an_enable_is_refused_while_a_reader_pinned_before_the_constructor_reads_its_relation_whole() {
    let got = enables(
        40,
        ADD_APPLE,
        ENABLE_APPLE,
        vec![reader("web-1", 1, vec![whole("auth::identities")])],
    );
    let why = refused(got).join("\n");
    assert!(why.contains("web-1"), "the refusal names no bundle: {why}");
    assert!(why.contains("released"), "{why}");
}

#[test]
fn an_enable_applies_beside_readers_that_cannot_reach_the_constructor() {
    let cases = [
        reader("users", 1, vec![whole("auth::users")]),
        reader("pinned-after", 2, vec![whole("auth::identities")]),
        reader("mail", 1, vec![follows("auth::identities", Some("Mail"))]),
        reader("fields", 1, vec![field("auth::identities", "user")]),
        Reader {
            hides_new: true,
            ..reader("hiding", 1, vec![whole("auth::identities")])
        },
    ];
    for (seed, r) in (41..).zip(cases) {
        let bundle = r.bundle.clone();
        let got = enables(seed, ADD_APPLE, ENABLE_APPLE, vec![r]);
        assert!(
            got.is_ok(),
            "seed {seed}: `{bundle}` blocks the enable: {got:?}"
        );
    }
}

#[test]
fn an_enable_is_refused_while_a_reader_follows_a_reference_that_admits_the_constructor() {
    for (seed, set) in [(50, None), (51, Some("OAuth"))] {
        let r = reader("api-1", 1, vec![follows("auth::identities", set)]);
        let why = refused(enables(seed, ADD_APPLE, ENABLE_APPLE, vec![r])).join("\n");
        assert!(why.contains("api-1"), "seed {seed}: {why}");
    }
}

#[test]
fn a_value_enable_is_refused_while_a_reader_pinned_before_it_reads_a_field_reaching_the_type() {
    let cases = [
        reader("field", 1, vec![field("auth::accounts", "provider")]),
        reader("option", 1, vec![field("auth::profiles", "backup")]),
        Reader {
            hides_new: true,
            ..reader("hiding", 1, vec![field("auth::accounts", "provider")])
        },
    ];
    for (seed, r) in (60..).zip(cases) {
        let bundle = r.bundle.clone();
        let why = refused(enables(seed, ADD_OKTA_TYPE, ENABLE_OKTA, vec![r])).join("\n");
        assert!(why.contains(&bundle), "seed {seed}: {why}");
    }
}

#[test]
fn a_value_enable_applies_beside_readers_of_no_field_reaching_the_type() {
    let cases = [
        reader("notes", 1, vec![field("auth::accounts", "note")]),
        reader("ranges", 1, vec![whole("auth::accounts")]),
        reader("pinned-after", 2, vec![whole("auth::accounts")]),
    ];
    for (seed, r) in (70..).zip(cases) {
        let bundle = r.bundle.clone();
        let got = enables(seed, ADD_OKTA_TYPE, ENABLE_OKTA, vec![r]);
        assert!(
            got.is_ok(),
            "seed {seed}: `{bundle}` blocks the enable: {got:?}"
        );
    }
}

#[test]
fn an_enable_refused_for_a_reader_applies_when_presented_again_after_its_release() {
    at_catalog(80, &[AUTH, ADD_APPLE], |catalog, pager, txns| {
        catalog
            .register(reader("web-1", 1, vec![whole("auth::identities")]))
            .expect("the reader registers");
        refused(catalog.migrate(pager, txns, ENABLE_APPLE));
        assert_eq!(2, catalog.source().version);
        catalog
            .release(pager, txns, "web-1")
            .expect("the reader is released");
        catalog
            .migrate(pager, txns, ENABLE_APPLE)
            .expect("the enable applies once its reader is released");
        assert_eq!(3, catalog.source().version);
    });
}

#[test]
fn deprecate_and_disable_are_refused_while_a_live_bundle_can_write_into_the_constructor() {
    let writer = |bundle: &str| Reader {
        writes: vec![("auth::identities".to_string(), "Google".to_string())],
        ..reader(bundle, 1, Vec::new())
    };
    for (seed, op) in [(90, "deprecate"), (91, "disable")] {
        let text = format!("module auth\n{op} identities.Google\n");
        let why = at_catalog(seed, &[AUTH], |catalog, pager, txns| {
            catalog
                .register(writer("app-1"))
                .expect("the reader registers");
            refused(catalog.migrate(pager, txns, &text)).join("\n")
        });
        assert!(why.contains("app-1"), "seed {seed}: {why}");
    }
    at_catalog(92, &[AUTH], |catalog, pager, txns| {
        catalog
            .register(writer("app-1"))
            .expect("the reader registers");
        catalog
            .migrate(pager, txns, "module auth\ndeprecate identities.EmailPass\n")
            .expect("a bundle that writes another constructor does not block");
    });
}

const NO_BLANK: &str =
    "module auth\nadd constraint no_blank = from u in users where u.name == \"\" select u\n";
const DROP_NO_BLANK: &str = "module auth\ndrop constraint no_blank\n";

fn pending(object: &str, dropped: u64, blocking: &[&str]) -> PendingDrop {
    PendingDrop {
        object: object.to_string(),
        dropped,
        blocking: blocking.iter().map(|b| (*b).to_string()).collect(),
    }
}

#[test]
fn a_drop_stays_pending_while_a_reader_pinned_to_a_version_with_the_constraint_is_live() {
    at_catalog(100, &[AUTH, NO_BLANK], |catalog, pager, txns| {
        catalog
            .register(reader("web-2", 2, vec![whole("auth::users")]))
            .expect("the reader registers");
        catalog
            .register(reader("web-1", 1, vec![whole("auth::users")]))
            .expect("the reader registers");
        catalog
            .migrate(pager, txns, DROP_NO_BLANK)
            .expect("the drop applies");
        assert_eq!(3, catalog.source().version);
        assert_eq!(
            vec![pending("auth::no_blank", 3, &["web-2"])],
            catalog.pending_drops()
        );
    });
}

#[test]
fn a_pending_dropped_constraint_is_carried_by_later_operators() {
    at_catalog(101, &[AUTH, NO_BLANK], |catalog, pager, txns| {
        catalog
            .register(reader("web-2", 2, vec![whole("auth::users")]))
            .expect("the reader registers");
        catalog
            .migrate(pager, txns, DROP_NO_BLANK)
            .expect("the drop applies");
        catalog
            .migrate(
                pager,
                txns,
                "module auth\nrename field User.name to full_name\n",
            )
            .expect("the rename carries the pending constraint");
        let why = refused(catalog.migrate(pager, txns, "module auth\ndrop field User.full_name\n"));
        assert!(
            why.iter().any(|w| w.contains("auth::no_blank")),
            "the refusal does not name the pending constraint: {why:?}"
        );
    });
}

#[test]
fn a_pending_drop_takes_effect_when_its_last_holding_reader_is_released() {
    at_catalog(102, &[AUTH, NO_BLANK], |catalog, pager, txns| {
        for b in ["web-2", "api-2"] {
            catalog
                .register(reader(b, 2, vec![whole("auth::users")]))
                .expect("the reader registers");
        }
        catalog
            .migrate(pager, txns, DROP_NO_BLANK)
            .expect("the drop applies");
        catalog
            .release(pager, txns, "web-2")
            .expect("the reader is released");
        assert_eq!(
            vec![pending("auth::no_blank", 3, &["api-2"])],
            catalog.pending_drops()
        );
        catalog
            .release(pager, txns, "api-2")
            .expect("the reader is released");
        assert!(catalog.pending_drops().is_empty());
        catalog
            .migrate(pager, txns, "module auth\ndrop field User.name\n")
            .expect("the drop has taken effect, so nothing reads the field");
    });
}

#[test]
fn a_pending_drop_takes_effect_at_the_next_embedded_open() {
    let mut sim = Sim::new(103, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[AUTH, NO_BLANK]);
        write(&db, AUTH_ROWS);
        db.close().expect("the database closes");
    });
    run(&mut sim, |rt| {
        let (pager, mut catalog, txns) = catalog_at(&rt);
        catalog
            .register(reader("web-2", 2, vec![whole("auth::users")]))
            .expect("the reader registers");
        catalog
            .migrate(&pager, &txns, DROP_NO_BLANK)
            .expect("the drop applies");
        assert_eq!(1, catalog.pending_drops().len());
        pager.close();
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[AUTH, NO_BLANK, DROP_NO_BLANK]);
        write(&db, r#"insert users { name: "" }"#);
    });
}

/// Run `body` over the catalog of a file at `[AUTH, NO_BLANK, DROP_NO_BLANK]` whose drop
/// `web-2` and `api-2` held pending in an earlier open of the catalog.
fn reopened_with_pending_drop<T>(
    seed: u64,
    body: impl FnOnce(&mut Catalog, &Pager<SimRuntime>, &Transactions<SimRuntime>) -> T,
) -> T {
    let mut sim = Sim::new(seed, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[AUTH, NO_BLANK]);
        write(&db, AUTH_ROWS);
        db.close().expect("the database closes");
    });
    run(&mut sim, |rt| {
        let (pager, mut catalog, txns) = catalog_at(&rt);
        for b in ["web-2", "api-2"] {
            catalog
                .register(reader(b, 2, vec![whole("auth::users")]))
                .expect("the reader registers");
        }
        catalog
            .migrate(&pager, &txns, DROP_NO_BLANK)
            .expect("the drop applies");
        pager.close();
    });
    run(&mut sim, |rt| {
        let (pager, mut catalog, txns) = catalog_at(&rt);
        let got = body(&mut catalog, &pager, &txns);
        pager.close();
        got
    })
}

#[test]
fn pending_drops_and_their_holders_carry_across_a_catalog_open() {
    reopened_with_pending_drop(120, |catalog, _, _| {
        assert_eq!(
            vec![pending("auth::no_blank", 3, &["api-2", "web-2"])],
            catalog.pending_drops()
        );
    });
}

#[test]
fn a_reopened_catalog_still_refuses_a_migration_that_leaves_a_pending_drop_without_an_image() {
    reopened_with_pending_drop(121, |catalog, pager, txns| {
        let why = refused(catalog.migrate(pager, txns, "module auth\ndrop field User.name\n"));
        assert!(
            why.iter().any(|w| w.contains("auth::no_blank")),
            "the refusal does not name the pending constraint: {why:?}"
        );
    });
}

#[test]
fn settling_keeps_the_holds_of_live_readers_and_releases_the_rest() {
    reopened_with_pending_drop(122, |catalog, pager, txns| {
        catalog
            .register(reader("web-2", 2, vec![whole("auth::users")]))
            .expect("the reader registers");
        catalog.settle(pager, txns).expect("the holds settle");
        assert_eq!(
            vec![pending("auth::no_blank", 3, &["web-2"])],
            catalog.pending_drops()
        );
    });
}

#[test]
fn a_hold_released_by_settling_does_not_return_at_the_next_open() {
    let mut sim = Sim::new(123, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[AUTH, NO_BLANK]);
        write(&db, AUTH_ROWS);
        db.close().expect("the database closes");
    });
    run(&mut sim, |rt| {
        let (pager, mut catalog, txns) = catalog_at(&rt);
        catalog
            .register(reader("web-2", 2, vec![whole("auth::users")]))
            .expect("the reader registers");
        catalog
            .migrate(&pager, &txns, DROP_NO_BLANK)
            .expect("the drop applies");
        pager.close();
    });
    run(&mut sim, |rt| {
        let (pager, mut catalog, txns) = catalog_at(&rt);
        catalog.settle(&pager, &txns).expect("the holds settle");
        assert!(catalog.pending_drops().is_empty());
        pager.close();
    });
    run(&mut sim, |rt| {
        let (pager, catalog, _) = catalog_at(&rt);
        assert!(catalog.pending_drops().is_empty());
        pager.close();
    });
}

#[test]
fn a_release_is_durable_across_a_catalog_open() {
    let mut sim = Sim::new(124, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, &[AUTH, NO_BLANK]);
        write(&db, AUTH_ROWS);
        db.close().expect("the database closes");
    });
    run(&mut sim, |rt| {
        let (pager, mut catalog, txns) = catalog_at(&rt);
        for b in ["web-2", "api-2"] {
            catalog
                .register(reader(b, 2, vec![whole("auth::users")]))
                .expect("the reader registers");
        }
        catalog
            .migrate(&pager, &txns, DROP_NO_BLANK)
            .expect("the drop applies");
        catalog
            .release(&pager, &txns, "web-2")
            .expect("the reader is released");
        pager.close();
    });
    run(&mut sim, |rt| {
        let (pager, catalog, _) = catalog_at(&rt);
        assert_eq!(
            vec![pending("auth::no_blank", 3, &["api-2"])],
            catalog.pending_drops()
        );
        pager.close();
    });
}

const UNIQUE_NAME: &str = "module auth\nadd unique (name) in users\n";
const DROP_UNIQUE_NAME: &str = "module auth\ndrop unique (name) in users\n";
const LABELS: &str = "module auth\nadd type Label = { name: Text, lang: Text }\nadd relation labels : Label key name\n";
const REKEY_LABELS: &str = "module auth\nrekey relation labels key (name, lang)\n";

/// An object of relation `relation` that migration `added` adds and migration `dropped` drops,
/// listed as `object` while its drop is pending.
struct Dropping {
    added: &'static str,
    dropped: &'static str,
    relation: &'static str,
    object: &'static str,
}

const UNIQUE: Dropping = Dropping {
    added: UNIQUE_NAME,
    dropped: DROP_UNIQUE_NAME,
    relation: "users",
    object: "auth::users unique (name)",
};

const OLD_KEY: Dropping = Dropping {
    added: LABELS,
    dropped: REKEY_LABELS,
    relation: "labels",
    object: "auth::labels key name",
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
    catalog.plan().relations[&r]
        .indexes
        .iter()
        .filter(|i| i.unique)
        .count()
}

fn stays_pending_while_a_reader_pinned_before_the_drop_is_live(seed: u64, d: &Dropping) {
    at_catalog(seed, &[AUTH, d.added], |catalog, pager, txns| {
        catalog
            .register(reader("web-2", 2, vec![d.whole()]))
            .expect("the reader registers");
        catalog
            .migrate(pager, txns, d.dropped)
            .expect("the drop applies");
        assert_eq!(3, catalog.source().version);
        assert_eq!(
            vec![pending(d.object, 3, &["web-2"])],
            catalog.pending_drops()
        );
        assert_eq!(2, unique_indexes(catalog, d.relation));
        assert_clean(pager, catalog, txns, 0);
    });
}

#[test]
fn a_dropped_unique_stays_pending_while_a_reader_pinned_before_the_drop_is_live() {
    stays_pending_while_a_reader_pinned_before_the_drop_is_live(104, &UNIQUE);
}

#[test]
fn a_relaxing_rekey_keeps_its_old_key_pending_while_a_reader_pinned_before_it_is_live() {
    stays_pending_while_a_reader_pinned_before_the_drop_is_live(130, &OLD_KEY);
}

fn applies_at_once_when_no_reader_holds_it(seed: u64, d: &Dropping) {
    at_catalog(seed, &[AUTH, d.added], |catalog, pager, txns| {
        catalog
            .migrate(pager, txns, d.dropped)
            .expect("the drop applies");
        assert!(catalog.pending_drops().is_empty());
        assert_eq!(1, unique_indexes(catalog, d.relation));
        assert_clean(pager, catalog, txns, 0);
    });
}

#[test]
fn a_dropped_unique_no_reader_holds_applies_at_once() {
    applies_at_once_when_no_reader_holds_it(125, &UNIQUE);
}

#[test]
fn a_relaxing_rekey_no_reader_holds_drops_its_old_key_at_once() {
    applies_at_once_when_no_reader_holds_it(131, &OLD_KEY);
}

fn frees_its_index_when_its_last_holder_is_released(seed: u64, d: &Dropping) {
    at_catalog(seed, &[AUTH, d.added], |catalog, pager, txns| {
        for b in ["web-2", "api-2"] {
            catalog
                .register(reader(b, 2, vec![d.whole()]))
                .expect("the reader registers");
        }
        catalog
            .migrate(pager, txns, d.dropped)
            .expect("the drop applies");
        catalog
            .release(pager, txns, "web-2")
            .expect("the reader is released");
        assert_eq!(2, unique_indexes(catalog, d.relation));
        catalog
            .release(pager, txns, "api-2")
            .expect("the reader is released");
        assert!(catalog.pending_drops().is_empty());
        assert_eq!(1, unique_indexes(catalog, d.relation));
        assert_clean(pager, catalog, txns, 0);
    });
}

#[test]
fn a_pending_unique_takes_effect_and_frees_its_index_when_its_last_holder_is_released() {
    frees_its_index_when_its_last_holder_is_released(126, &UNIQUE);
}

#[test]
fn a_pending_old_key_takes_effect_and_frees_its_index_when_its_last_holder_is_released() {
    frees_its_index_when_its_last_holder_is_released(132, &OLD_KEY);
}

#[test]
fn a_pending_unique_is_carried_by_later_operators_and_refuses_one_that_leaves_it_no_image() {
    at_catalog(127, &[AUTH, UNIQUE_NAME], |catalog, pager, txns| {
        catalog
            .register(reader("web-2", 2, vec![whole("auth::users")]))
            .expect("the reader registers");
        catalog
            .migrate(pager, txns, DROP_UNIQUE_NAME)
            .expect("the drop applies");
        catalog
            .migrate(
                pager,
                txns,
                "module auth\nrename field User.name to full_name\n",
            )
            .expect("the rename carries the pending unique");
        assert_eq!(2, unique_indexes(catalog, "users"));
        let why = refused(catalog.migrate(pager, txns, "module auth\ndrop field User.full_name\n"));
        assert!(
            why.iter().any(|w| w.contains("auth::users unique (name)")),
            "the refusal does not name the pending unique: {why:?}"
        );
    });
}

#[test]
fn a_pending_old_key_is_carried_by_later_operators_and_refuses_one_that_leaves_it_no_image() {
    at_catalog(133, &[AUTH, LABELS], |catalog, pager, txns| {
        catalog
            .register(reader("web-2", 2, vec![whole("auth::labels")]))
            .expect("the reader registers");
        catalog
            .migrate(pager, txns, REKEY_LABELS)
            .expect("the rekey applies");
        catalog
            .migrate(
                pager,
                txns,
                "module auth\nrename field Label.name to title\n",
            )
            .expect("the rename carries the pending old key");
        assert_eq!(2, unique_indexes(catalog, "labels"));
        assert_clean(pager, catalog, txns, 0);
        let why = refused(catalog.migrate(
            pager,
            txns,
            "module auth\nrekey relation labels key lang\ndrop field Label.title\n",
        ));
        assert!(
            why.iter()
                .any(|w| w.contains("auth::labels key name") && w.contains("web-2")),
            "the refusal does not name the pending old key and its holder: {why:?}"
        );
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

fn carries_across_a_catalog_open_until_released(seed: u64, d: &Dropping) {
    let mut sim = Sim::new(seed, Config::default());
    reopened_with_pending(&mut sim, d, |_, catalog, pager, txns| {
        assert_eq!(
            vec![pending(d.object, 3, &["web-2"])],
            catalog.pending_drops()
        );
        assert_eq!(2, unique_indexes(catalog, d.relation));
        assert_clean(pager, catalog, txns, 0);
        catalog.settle(pager, txns).expect("the holds settle");
        assert!(catalog.pending_drops().is_empty());
        assert_eq!(1, unique_indexes(catalog, d.relation));
    });
    run(&mut sim, |rt| {
        let (pager, catalog, txns) = catalog_at(&rt);
        assert!(catalog.pending_drops().is_empty());
        assert_eq!(1, unique_indexes(&catalog, d.relation));
        assert_clean(&pager, &catalog, &txns, 0);
        pager.close();
    });
}

#[test]
fn a_pending_unique_and_its_index_carry_across_a_catalog_open_until_released() {
    carries_across_a_catalog_open_until_released(128, &UNIQUE);
}

#[test]
fn a_pending_old_key_and_its_index_carry_across_a_catalog_open_until_released() {
    carries_across_a_catalog_open_until_released(134, &OLD_KEY);
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

#[test]
fn a_relaxing_rekey_that_drops_its_old_key_field_is_refused_while_a_reader_holds_the_old_key() {
    let rekey =
        "module auth\nrekey relation users key name\n  carry Identity.user\ndrop field User.id\n";
    at_catalog(106, &[AUTH], |catalog, pager, txns| {
        catalog
            .register(reader("web-1", 1, vec![field("auth::users", "name")]))
            .expect("the reader registers");
        let why = refused(catalog.migrate(pager, txns, rekey)).join("\n");
        assert!(why.contains("web-1"), "{why}");
        catalog
            .release(pager, txns, "web-1")
            .expect("the reader is released");
        catalog
            .migrate(pager, txns, rekey)
            .expect("the rekey applies once no reader holds the old key");
    });
}

#[test]
fn a_dropped_check_stays_pending_like_a_constraint() {
    let check = "module auth\nadd check named on users (scalar_len(name) > 0)\n";
    let drop = "module auth\ndrop check named\n";
    at_catalog(107, &[AUTH, check], |catalog, pager, txns| {
        catalog
            .register(reader("web-2", 2, vec![whole("auth::users")]))
            .expect("the reader registers");
        catalog
            .migrate(pager, txns, drop)
            .expect("the drop applies");
        assert_eq!(
            vec![pending("auth::named", 3, &["web-2"])],
            catalog.pending_drops()
        );
    });
}

const DISABLE_APPLE: &str = "module auth\ndisable identities.Apple\n";

#[test]
fn a_revert_that_enables_is_refused_while_a_reader_pinned_before_the_constructor_reads_it() {
    let renamed =
        "module auth\ndisable identities.Apple\nrename constructor Identity.Apple to Pear\n";
    for (seed, last) in [(110, DISABLE_APPLE), (111, renamed)] {
        let why = at_catalog(
            seed,
            &[AUTH, ADD_APPLE, ENABLE_APPLE, last],
            |catalog, pager, txns| {
                catalog
                    .register(reader("web-1", 1, vec![whole("auth::identities")]))
                    .expect("the reader registers");
                let why = refused(catalog.migrate(pager, txns, "module auth\nrevert 0004\n"));
                assert_eq!(4, catalog.source().version, "seed {seed}");
                why.join("\n")
            },
        );
        assert!(why.contains("web-1"), "seed {seed}: {why}");
        assert!(why.contains("released"), "seed {seed}: {why}");
    }
}

#[test]
fn a_revert_that_disables_is_refused_while_a_live_bundle_can_write_into_the_constructor() {
    let why = at_catalog(
        112,
        &[AUTH, ADD_APPLE, ENABLE_APPLE],
        |catalog, pager, txns| {
            catalog
                .register(Reader {
                    writes: vec![("auth::identities".to_string(), "Apple".to_string())],
                    ..reader("app-3", 3, Vec::new())
                })
                .expect("the reader registers");
            refused(catalog.migrate(pager, txns, "module auth\nrevert 0003\n")).join("\n")
        },
    );
    assert!(why.contains("app-3"), "{why}");
}

#[test]
fn a_revert_that_drops_a_constraint_a_reader_holds_keeps_it_pending() {
    at_catalog(113, &[AUTH, NO_BLANK], |catalog, pager, txns| {
        catalog
            .register(reader("web-2", 2, vec![whole("auth::users")]))
            .expect("the reader registers");
        catalog
            .migrate(pager, txns, "module auth\nrevert 0002\n")
            .expect("the revert applies");
        assert_eq!(
            vec![pending("auth::no_blank", 3, &["web-2"])],
            catalog.pending_drops()
        );
        let why = refused(catalog.migrate(pager, txns, "module auth\ndrop field User.name\n"));
        assert!(
            why.iter().any(|w| w.contains("auth::no_blank")),
            "the refusal does not name the pending constraint: {why:?}"
        );
        catalog
            .release(pager, txns, "web-2")
            .expect("the reader is released");
        assert!(catalog.pending_drops().is_empty());
        catalog
            .migrate(pager, txns, "module auth\ndrop field User.name\n")
            .expect("the drop has taken effect, so nothing reads the field");
    });
}

#[test]
fn a_revert_that_drops_a_check_a_reader_holds_keeps_it_pending() {
    let check = "module auth\nadd check named on users (scalar_len(name) > 0)\n";
    at_catalog(114, &[AUTH, check], |catalog, pager, txns| {
        catalog
            .register(reader("web-2", 2, vec![whole("auth::users")]))
            .expect("the reader registers");
        catalog
            .migrate(pager, txns, "module auth\nrevert 0002\n")
            .expect("the revert applies");
        assert_eq!(
            vec![pending("auth::named", 3, &["web-2"])],
            catalog.pending_drops()
        );
    });
}

#[test]
fn a_pending_drop_is_carried_by_the_operators_after_it_in_its_own_migration() {
    let drop_and_rename =
        "module auth\ndrop constraint no_blank\nrename field User.name to full_name\n";
    at_catalog(115, &[AUTH, NO_BLANK], |catalog, pager, txns| {
        catalog
            .register(reader("web-2", 2, vec![whole("auth::users")]))
            .expect("the reader registers");
        catalog
            .migrate(pager, txns, drop_and_rename)
            .expect("the rename carries the pending constraint");
        let why = refused(catalog.migrate(pager, txns, "module auth\ndrop field User.full_name\n"));
        assert!(
            why.iter().any(|w| w.contains("auth::no_blank")),
            "the refusal does not name the pending constraint: {why:?}"
        );
    });
}

#[test]
fn a_pending_dropped_constraint_is_rechecked_against_later_operators() {
    let matching = r#"module auth
add constraint no_blank_login = from i in identities
  where match i
    | EmailPass e -> e.email == ""
    | Google g    -> g.gid == ""
  select i
"#;
    let drop = "module auth\ndrop constraint no_blank_login\n";
    at_catalog(108, &[AUTH, matching], |catalog, pager, txns| {
        catalog
            .register(reader("web-2", 2, vec![whole("auth::identities")]))
            .expect("the reader registers");
        catalog
            .migrate(pager, txns, drop)
            .expect("the drop applies");
        let why = refused(catalog.migrate(pager, txns, ADD_APPLE));
        assert!(
            why.iter().any(|w| w.contains("auth::no_blank_login")),
            "the refusal does not name the pending constraint: {why:?}"
        );
    });
}

#[test]
fn a_reader_naming_nothing_at_its_version_is_not_registered() {
    at_catalog(105, &[AUTH], |catalog, pager, txns| {
        assert_eq!(
            vec!["bundle bad at version 1: there is no relation auth::nothing"],
            refused(catalog.register(reader("bad", 1, vec![whole("auth::nothing")])))
        );
        assert_eq!(
            vec!["bundle future is pinned at version 2, which the log does not hold"],
            refused(catalog.register(reader("future", 2, Vec::new())))
        );
        assert_eq!(
            vec!["bundle unknown is not live"],
            refused(catalog.release(pager, txns, "unknown"))
        );
    });
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
