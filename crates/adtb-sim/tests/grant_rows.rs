//! The `Log` rows of `catalog::grants`, beside its `Deployment` rows: their ids, their targets,
//! the refusal of a grant to a missing role, dormant rows on replay, and crashes.

use adtb_simkit::api;
use adtb_simkit::grants;
use adtb_simkit::harness;

use std::collections::{BTreeMap, BTreeSet};

use adtb::catalog::{Catalog, Dormant, Error as CatalogError, layout, values};
use adtb::key::Value;
use adtb::mvcc::Transactions;
use adtb::storage::Pager;
use adtb::verify::{self, Depth};
use adtb::{Capability, Database, Error, Log, OpenError, RejectReason};
use adtb_lang::evolve::Granted;
use adtb_sim::{Config, Sim, SimRuntime};

use api::{once, options, query, run};
use harness::{finished, options as pager_options};

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
";

const FRAMES: usize = 64;

/// A `Log` row's id, grant option and dormancy.
type LogRow = (i64, bool, bool);

/// A reopened catalog's version, `Log` rows and `Deployment` row ids.
type Seen = (u64, Vec<LogRow>, Vec<i64>);

fn open(rt: &SimRuntime, log: &[&str]) -> Result<Database<SimRuntime>, OpenError> {
    Database::open_in(
        rt.clone(),
        "app.adt",
        &Log::new(log.iter().copied()),
        &options(2),
    )
}

fn opened(rt: &SimRuntime, log: &[&str]) -> Database<SimRuntime> {
    open(rt, log).unwrap_or_else(|e| panic!("the database opens at {}: {e:?}", log.len()))
}

/// Returns each row of `catalog::grants`, printed, by its id.
fn by_id(db: &Database<SimRuntime>) -> BTreeMap<i64, String> {
    query(db, "from g in catalog::grants select g")
        .into_iter()
        .map(|row| {
            let id = row
                .split_once("id: grants.key(")
                .and_then(|(_, r)| r.split(')').next())
                .and_then(|n| n.parse().ok())
                .unwrap_or_else(|| panic!("a grant row starts with its id: {row}"));
            (id, row)
        })
        .collect()
}

/// Returns the id of the one row of `rows` holding each of `parts`.
fn id_of(rows: &BTreeMap<i64, String>, parts: &[&str]) -> i64 {
    let found: Vec<i64> = rows
        .iter()
        .filter(|(_, r)| parts.iter().all(|p| r.contains(p)))
        .map(|(&id, _)| id)
        .collect();
    assert_eq!(1, found.len(), "one row holds {parts:?}: {rows:#?}");
    found[0]
}

fn refusal(got: Result<Database<SimRuntime>, OpenError>) -> (u64, String) {
    match got {
        Err(OpenError::MigrationRefused {
            version,
            reason:
                Error::Rejected {
                    reason: RejectReason::SchemaRefused { detail, .. },
                },
        }) => (version, detail),
        Err(other) => panic!("not a refused migration: {other:?}"),
        Ok(_) => panic!("the migration applies"),
    }
}

#[test]
fn log_and_deployment_ids_interleave_without_collision_across_every_change() {
    let grant =
        "module shop\ngrant read customers to web\ngrant read payments.Card (last4) to support\n";
    let revoke = "module shop\nrevoke read customers from web\n";
    let regrant = "module shop\ngrant read customers to web\n";
    let rename = "module shop\nrename relation customers to clients\nrename field Payment.Card.last4 to digits\n";
    let option = "module shop\ngrant read clients to web with grant option\n";
    let split = r"module shop
split constructor Payment.Card into Visa { digits: Text } | Other { digits: Text }
  by match
    | Card { digits } -> Visa { digits: digits }
";
    let paid = "module shop
grant delete clients to web
add constraint paid =
  from c in clients
  where not exists p in payments where p.customer == key c
  select c
";
    let delete = "module shop\ngrant delete clients to web\n";
    let mut sim = Sim::new(140, Config::default());
    let first = run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        let authority = db.role_authority();
        authority.create_role("web").expect("web");
        authority.create_role("support").expect("support");
        authority
            .grant_capability("web", Capability::Assert, false)
            .expect("assert");
        db.query_text(
            "insert customers { name: \"ann\" }",
            &[],
            &adtb::CallOptions::default(),
        )
        .expect("a customer");
        by_id(&db)
    });
    let assert = id_of(&first, &["Deployment", "\"assert\""]);
    let log = [SHOP, grant];
    let second = run(&mut sim, |rt| {
        let db = opened(&rt, &log);
        db.role_authority()
            .grant_capability("support", Capability::Monitor, false)
            .expect("monitor");
        by_id(&db)
    });
    let read = id_of(
        &second,
        &["Log", "\"shop::customers frozen (id, name)\"", "\"web\""],
    );
    let card = id_of(
        &second,
        &["Log", "\"shop::payments.Card (Card.last4)\"", "\"support\""],
    );
    let monitor = id_of(&second, &["Deployment", "\"monitor\""]);
    assert_eq!(first.get(&assert), second.get(&assert));
    assert!(
        assert < read.min(card) && read.max(card) < monitor,
        "{second:#?}"
    );
    let third = run(&mut sim, |rt| by_id(&opened(&rt, &[SHOP, grant, revoke])));
    assert_eq!(
        BTreeSet::from([assert, card, monitor]),
        third.keys().copied().collect()
    );
    let fourth = run(&mut sim, |rt| {
        by_id(&opened(&rt, &[SHOP, grant, revoke, regrant]))
    });
    let again = id_of(&fourth, &["Log", "\"web\""]);
    assert!(again > monitor, "a regrant takes a new id: {fourth:#?}");
    let log = [SHOP, grant, revoke, regrant, rename, option, split];
    let fifth = run(&mut sim, |rt| by_id(&opened(&rt, &log)));
    assert_eq!(
        BTreeSet::from([assert, card, monitor, again]),
        fifth.keys().copied().collect(),
        "a rename, an option change and a carry keep each row's id"
    );
    assert!(fifth[&again].contains("\"shop::clients frozen (id, name)\""));
    assert!(fifth[&again].contains("grant_option: True"));
    assert!(
        fifth[&card].contains("\"shop::payments.Visa (Visa.digits)\""),
        "{}",
        fifth[&card]
    );
    run(&mut sim, |rt| {
        let (version, detail) = refusal(open(
            &rt,
            &[SHOP, grant, revoke, regrant, rename, option, split, paid],
        ));
        assert_eq!(8, version);
        assert!(detail.contains("paid"), "{detail}");
    });
    let sixth = run(&mut sim, |rt| {
        let db = opened(&rt, &log);
        assert_eq!(fifth, by_id(&db), "a refused migration publishes no row");
        db.role_authority()
            .grant_capability("web", Capability::Cancel, false)
            .expect("cancel");
        by_id(&db)
    });
    let cancel = id_of(&sixth, &["Deployment", "\"cancel\""]);
    assert!(cancel > again);
    let full = [SHOP, grant, revoke, regrant, rename, option, split, delete];
    let seventh = run(&mut sim, |rt| by_id(&opened(&rt, &full)));
    let deleted = id_of(&seventh, &["\"delete\""]);
    assert!(deleted > cancel, "{seventh:#?}");
    let ids: BTreeSet<i64> = [first, second, third, fourth, fifth, sixth, seventh.clone()]
        .iter()
        .flat_map(|rows| rows.keys().copied())
        .collect();
    assert_eq!(
        ids,
        BTreeSet::from([assert, read, card, monitor, again, cancel, deleted]),
        "no id names two grants"
    );
    let reopened = run(&mut sim, |rt| by_id(&opened(&rt, &full)));
    assert_eq!(seventh, reopened);
}

#[test]
fn a_grant_to_a_missing_role_is_refused_and_publishes_nothing() {
    let ghost = "module shop\ngrant read customers to ghost\n";
    let audit = "from a in catalog::audit select a";
    let mut sim = Sim::new(141, Config::default());
    let before = run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        db.role_authority().create_role("web").expect("web");
        (query(&db, audit), by_id(&db))
    });
    run(&mut sim, |rt| {
        let (version, detail) = refusal(open(&rt, &[SHOP, ghost]));
        assert_eq!(2, version);
        assert!(detail.contains("role `ghost`"), "{detail}");
    });
    run(&mut sim, |rt| {
        let db = opened(&rt, &[SHOP]);
        assert_eq!(before, (query(&db, audit), by_id(&db)));
        assert_eq!(
            vec!["1"],
            query(&db, "count(m for m in catalog::migrations)")
        );
    });
}

/// Returns the id, grant option and dormancy of each `Log` row of `catalog::grants` the latest
/// commit holds, and the id of each `Deployment` row.
fn stored(
    catalog: &Catalog,
    pager: &Pager<SimRuntime>,
    txns: &Transactions<SimRuntime>,
) -> (Vec<LogRow>, Vec<i64>) {
    let schema = catalog.schema();
    let (r, c) = grants::catalog_relation(schema, "grants", "RoleGrant");
    let names = layout::row(schema, r, c);
    let at = |n: &str| names.iter().position(|(f, _)| f == n).expect("a field");
    let yes = values::tag(schema, "prelude", "Bool", "True");
    let log = values::tag(schema, "catalog", "GrantKind", "Log");
    let mut logged = Vec::new();
    let mut deployed = Vec::new();
    for (_, row) in catalog.committed_rows(pager, txns, r).expect("grant rows") {
        let Value::I64(id) = row[at("id")] else {
            panic!("a grant id is an I64");
        };
        if row[at("kind")] == log {
            logged.push((
                id,
                row[at("grant_option")] == yes,
                row[at("dormant")] == yes,
            ));
        } else {
            deployed.push(id);
        }
    }
    logged.sort_unstable();
    deployed.sort_unstable();
    (logged, deployed)
}

#[test]
fn replay_keeps_a_dormant_row_until_a_later_migration_grants_the_privilege_again() {
    let mut sim = Sim::new(142, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &pager_options(FRAMES)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        assert_eq!(Ok(Vec::new()), catalog.replay(&pager, &txns, SHOP));
        let dormant = catalog
            .replay(
                &pager,
                &txns,
                "module shop\ngrant read customers to ghost\n",
            )
            .expect("replay applies a grant to a missing role");
        assert_eq!(
            vec![Dormant {
                id: 1,
                grantee: "ghost".into(),
                privilege: "read".into(),
                target: "shop::customers frozen (id, name)".into(),
            }],
            dormant
        );
        assert_eq!(
            Err(CatalogError::Refused(vec![
                "a grant names role `ghost`, which does not exist".into()
            ])),
            catalog.migrate(
                &pager,
                &txns,
                "module shop\ngrant insert customers to ghost\n"
            )
        );
        assert_eq!(2, catalog.source().version);
        grants::create_role(&catalog, &pager, &txns, 1, "ghost");
        assert_eq!(
            (vec![(1, false, true)], Vec::new()),
            stored(&catalog, &pager, &txns),
            "creating the role does not activate a dormant row"
        );
        catalog
            .migrate(
                &pager,
                &txns,
                "module shop\ngrant insert customers to ghost\n",
            )
            .expect("a grant the role now exists for applies");
        let (logged, _) = stored(&catalog, &pager, &txns);
        assert_eq!(2, logged.len(), "{logged:?}");
        assert_eq!(
            (1, false, true),
            logged[0],
            "another privilege leaves it dormant"
        );
        assert!(logged[1].0 > 1 && !logged[1].2, "{logged:?}");
        drop(catalog);
        drop(txns);
        pager.close();
    });
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &pager_options(FRAMES)).expect("reopen");
        let (mut catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
        let (before, _) = stored(&catalog, &pager, &txns);
        assert_eq!((1, false, true), before[0], "reopening activates nothing");
        catalog
            .migrate(
                &pager,
                &txns,
                "module shop\ngrant read customers to ghost with grant option\n",
            )
            .expect("the regrant applies");
        let (after, _) = stored(&catalog, &pager, &txns);
        assert_eq!(
            vec![(1, true, false), before[1]],
            after,
            "the regrant activates the row and keeps its id"
        );
    });
}

#[test]
fn every_log_target_reads_back_to_its_grant() {
    let mut sim = Sim::new(143, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &pager_options(FRAMES)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        catalog.migrate(&pager, &txns, SHOP).expect("the schema");
        grants::create_role(&catalog, &pager, &txns, 1, "web");
        let carried = r#"module shop
add set Late of payments = Invoice
define fn shout (s: Text) -> Text = s ++ s
grant read payments.Late to web
grant update payments.(Card | Invoice) (customer) to web
grant retag payments.Card -> payments.(Card | Invoice) to web
grant insert customers to web
grant delete payments.Invoice to web
grant call fn shout to web
grant read customers (name) to web
split constructor Payment.Card into Visa { last4: Text } | Other { last4: Text }
  by match
    | Card { last4: "0000" } -> Other { last4: "0000" }
    | Card { last4 } -> Visa { last4: last4 }
rename relation customers to clients
"#;
        catalog
            .migrate(&pager, &txns, carried)
            .expect("the grants apply");
        let schema = catalog.schema();
        let texts: Vec<(Granted, String)> = catalog
            .source()
            .grants
            .iter()
            .map(|g| {
                let t = g.privilege.target_text(schema).expect("a live target");
                (g.privilege.clone(), t)
            })
            .collect();
        assert_eq!(7, texts.len());
        for (g, t) in &texts {
            assert_eq!(
                Some(g.clone()),
                Granted::read_target(schema, g.privilege_name(), t),
                "{t}"
            );
        }
        let retag = texts
            .iter()
            .find(|(g, _)| g.privilege_name() == "retag")
            .map(|(_, t)| t.as_str());
        assert_eq!(
            Some(
                "shop::payments.(Visa | Other) -> shop::payments.(Invoice | Visa | Other) except (Visa -> Other, Other -> Visa)"
            ),
            retag
        );
    });
}

/// Returns the version, the `Log` rows and the `Deployment` ids of the catalog `files` holds
/// once it opens, after checking that it verifies clean.
fn reopened(seed: u64, files: BTreeMap<String, Vec<u8>>) -> Seen {
    let mut sim = Sim::with_files(seed, Config::default(), files);
    let mut got = None;
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &pager_options(FRAMES)).expect("reopen");
        let (mut catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
        let clog = txns.commit_log();
        let report = verify::verify_with(&pager, &catalog.inventory(&clog), Depth::STRUCTURE);
        assert!(
            report.findings.is_empty(),
            "seed {seed}: {:?}",
            report.findings
        );
        drop(clog);
        let (logged, deployed) = stored(&catalog, &pager, &txns);
        let version = catalog.source().version;
        if version > 0 {
            catalog
                .migrate(
                    &pager,
                    &txns,
                    "module shop\ngrant delete customers to web\n",
                )
                .unwrap_or_else(|e| panic!("seed {seed}: a later grant applies: {e:?}"));
            let (after, _) = stored(&catalog, &pager, &txns);
            let fresh: Vec<i64> = after
                .iter()
                .map(|r| r.0)
                .filter(|id| !logged.iter().any(|l| l.0 == *id))
                .collect();
            let held = logged.iter().map(|l| l.0).chain(deployed.iter().copied());
            let highest = held.max().unwrap_or(0);
            assert!(
                !fresh.is_empty() && fresh.iter().all(|&f| f > highest),
                "seed {seed}: a later grant reuses no id: {logged:?} {deployed:?} {after:?}"
            );
        }
        got = Some((version, logged, deployed));
    });
    got.expect("the catalog opened")
}

/// Run the recorded `body` after `setup`, then reopen an image at every flush `body` made and
/// return what [`reopened`] finds there.
fn at_every_flush(
    seed: u64,
    setup: impl FnOnce(&SimRuntime, &Pager<SimRuntime>, &mut Catalog, &Transactions<SimRuntime>),
    body: impl FnOnce(&Pager<SimRuntime>, &mut Catalog, &Transactions<SimRuntime>),
) -> Vec<Seen> {
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    let mut start = 0;
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &pager_options(FRAMES)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt.clone(), &pager).expect("the catalog");
        setup(&rt, &pager, &mut catalog, &txns);
        start = rt.recorded();
        body(&pager, &mut catalog, &txns);
    });
    let recording = sim.recording();
    let flushes: Vec<usize> = recording
        .flushes()
        .into_iter()
        .filter(|&f| f >= start)
        .collect();
    assert!(!flushes.is_empty(), "seed {seed}: the body flushes");
    flushes
        .into_iter()
        .map(|at| {
            let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
            reopened(seed + at as u64, files)
        })
        .collect()
}

fn shop_with_web(
    _: &SimRuntime,
    pager: &Pager<SimRuntime>,
    catalog: &mut Catalog,
    txns: &Transactions<SimRuntime>,
) {
    catalog.migrate(pager, txns, SHOP).expect("the schema");
    grants::create_role(catalog, pager, txns, 1, "web");
    grants::deployment_grant(catalog, pager, txns, 1, "web", "assert");
}

#[test]
fn a_crash_at_every_flush_of_a_migration_with_grants_keeps_both_row_classes() {
    let seen = at_every_flush(
        144,
        |rt, pager, catalog, txns| {
            shop_with_web(rt, pager, catalog, txns);
            catalog.counters(pager).expect("the counters");
        },
        |pager, catalog, txns| {
            catalog
                .migrate(
                    pager,
                    txns,
                    "module shop\ngrant read customers to web\ngrant insert customers to web\n",
                )
                .expect("the grants apply");
        },
    );
    for (version, logged, deployed) in &seen {
        assert_eq!(&vec![1], deployed, "the deployment row survives");
        match version {
            1 => assert!(logged.is_empty(), "{logged:?}"),
            2 => {
                assert_eq!(2, logged.len(), "{logged:?}");
                assert!(logged.iter().all(|l| l.0 > 1 && !l.2), "{logged:?}");
            }
            v => panic!("version {v}"),
        }
    }
    assert_eq!(Some(2), seen.last().map(|s| s.0));
}

#[test]
fn a_crash_at_every_flush_of_naming_the_counters_page_frees_or_names_it() {
    let seen = at_every_flush(145, shop_with_web, |pager, catalog, _| {
        catalog.counters(pager).expect("the counters");
    });
    assert!(seen.iter().all(|(v, _, d)| *v == 1 && d == &vec![1]));
}

#[test]
fn a_crash_at_every_flush_of_recording_a_legacy_grant_keeps_or_writes_its_row() {
    let seed = 146;
    let mut sim = Sim::new(seed, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &pager_options(FRAMES)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt.clone(), &pager).expect("the catalog");
        shop_with_web(&rt, &pager, &mut catalog, &txns);
        catalog
            .migrate(&pager, &txns, "module shop\ngrant read customers to web\n")
            .expect("the grant applies");
        let (r, c) = grants::catalog_relation(catalog.schema(), "grants", "RoleGrant");
        let kind = layout::row(catalog.schema(), r, c)
            .iter()
            .position(|(n, _)| n == "kind")
            .expect("RoleGrant declares kind");
        let log = values::tag(catalog.schema(), "catalog", "GrantKind", "Log");
        let rows: Vec<(u32, Vec<Value>)> = catalog
            .committed_rows(&pager, &txns, r)
            .expect("grant rows")
            .into_iter()
            .filter(|(_, row)| row[kind] == log)
            .collect();
        assert_eq!(1, rows.len());
        catalog
            .deploy(
                &pager,
                &txns,
                &[adtb::catalog::Rewrite {
                    relation: r,
                    deleted: rows,
                    inserted: Vec::new(),
                }],
            )
            .expect("the legacy fixture drops the row");
        drop(catalog);
        drop(txns);
        pager.close();
    });
    sim.record();
    let mut start = 0;
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &pager_options(FRAMES)).expect("reopen");
        let (mut catalog, txns) = Catalog::open(rt.clone(), &pager).expect("the catalog opens");
        assert_eq!(None, catalog.source().grants[0].row);
        start = rt.recorded();
        let dormant = catalog
            .record_grants(&pager, &txns)
            .expect("the grant is recorded");
        assert!(dormant.is_empty(), "web exists: {dormant:?}");
        assert!(catalog.source().grants[0].row.is_some_and(|id| id > 2));
    });
    let recording = sim.recording();
    let flushes: Vec<usize> = recording
        .flushes()
        .into_iter()
        .filter(|&f| f >= start)
        .collect();
    assert!(!flushes.is_empty());
    for at in flushes {
        let files = recording.images(at, 1, seed + at as u64).swap_remove(0);
        let (version, logged, deployed) = reopened(seed + at as u64, files);
        assert_eq!((2, vec![1]), (version, deployed));
        assert!(logged.len() <= 1, "{logged:?}");
    }
}

/// Returns a catalog at `SHOP` whose log then grants `read` on customers to the missing role
/// `ghost`, replayed so the grant's row, id 1, is dormant.
fn dormant_ghost(rt: &SimRuntime) -> (Catalog, Pager<SimRuntime>, Transactions<SimRuntime>) {
    let pager = Pager::create(rt.clone(), &pager_options(FRAMES)).expect("create");
    let (mut catalog, txns) = Catalog::create(rt.clone(), &pager).expect("the catalog");
    catalog.replay(&pager, &txns, SHOP).expect("the schema");
    catalog
        .replay(
            &pager,
            &txns,
            "module shop\ngrant read customers to ghost\n",
        )
        .expect("the dormant grant");
    assert_eq!(
        (vec![(1, false, true)], Vec::new()),
        stored(&catalog, &pager, &txns)
    );
    (catalog, pager, txns)
}

fn refused_for_ghost(got: Result<(), CatalogError>) {
    assert_eq!(
        Err(CatalogError::Refused(vec![
            "a grant names role `ghost`, which does not exist".into()
        ])),
        got
    );
}

#[test]
fn a_grant_operator_naming_a_missing_role_is_refused_even_when_it_merges() {
    let mut sim = Sim::new(148, Config::default());
    finished(&mut sim, |rt| {
        let (mut catalog, pager, txns) = dormant_ghost(&rt);
        refused_for_ghost(catalog.migrate(
            &pager,
            &txns,
            "module shop\ngrant read customers to ghost with grant option\n",
        ));
        refused_for_ghost(catalog.migrate(
            &pager,
            &txns,
            "module shop\ngrant insert customers to ghost\nrevoke insert customers from ghost\n",
        ));
        assert_eq!(2, catalog.source().version);
        assert_eq!(
            (vec![(1, false, true)], Vec::new()),
            stored(&catalog, &pager, &txns)
        );
        catalog
            .migrate(
                &pager,
                &txns,
                "module shop\nrevoke read customers from ghost\n",
            )
            .expect("a revoke may name a missing role");
        assert_eq!((Vec::new(), Vec::new()), stored(&catalog, &pager, &txns));
    });
}

#[test]
fn a_merge_of_an_active_and_a_dormant_grant_keeps_the_active_row() {
    let mut sim = Sim::new(149, Config::default());
    finished(&mut sim, |rt| {
        let (mut catalog, pager, txns) = dormant_ghost(&rt);
        grants::create_role(&catalog, &pager, &txns, 1, "ghost");
        catalog
            .migrate(
                &pager,
                &txns,
                "module shop\nadd field Customer.email: Text fill \"\"\n",
            )
            .expect("the field");
        catalog
            .migrate(
                &pager,
                &txns,
                "module shop\ngrant read customers to ghost\n",
            )
            .expect("a grant over the new field set");
        let (before, _) = stored(&catalog, &pager, &txns);
        assert_eq!(2, before.len(), "{before:?}");
        let active = before[1];
        assert!(!active.2, "{before:?}");
        catalog
            .migrate(&pager, &txns, "module shop\ndrop field Customer.email\n")
            .expect("the drop merges the two grants");
        assert_eq!((vec![active], Vec::new()), stored(&catalog, &pager, &txns));
    });
}

#[test]
fn a_merge_of_two_dormant_grants_keeps_one_dormant_row() {
    let mut sim = Sim::new(150, Config::default());
    finished(&mut sim, |rt| {
        let (mut catalog, pager, txns) = dormant_ghost(&rt);
        catalog
            .replay(
                &pager,
                &txns,
                "module shop\nadd field Customer.email: Text fill \"\"\n",
            )
            .expect("the field");
        catalog
            .replay(
                &pager,
                &txns,
                "module shop\ngrant read customers to ghost\n",
            )
            .expect("a second dormant grant");
        let (before, _) = stored(&catalog, &pager, &txns);
        assert_eq!(2, before.len(), "{before:?}");
        catalog
            .replay(&pager, &txns, "module shop\ndrop field Customer.email\n")
            .expect("the drop merges the two grants");
        assert_eq!(
            (vec![(1, false, true)], Vec::new()),
            stored(&catalog, &pager, &txns)
        );
    });
}

#[test]
fn reverting_a_revoke_restores_the_grant_as_dormant_as_it_was_across_a_reopen() {
    let mut sim = Sim::new(151, Config::default());
    finished(&mut sim, |rt| {
        let (mut catalog, pager, txns) = dormant_ghost(&rt);
        grants::create_role(&catalog, &pager, &txns, 1, "ghost");
        catalog
            .migrate(
                &pager,
                &txns,
                "module shop\nrevoke read customers from ghost\n",
            )
            .expect("the revoke");
        catalog
            .migrate(
                &pager,
                &txns,
                "module shop\ngrant insert customers to ghost\n",
            )
            .expect("an active grant");
        catalog
            .migrate(
                &pager,
                &txns,
                "module shop\nrevoke insert customers from ghost\n",
            )
            .expect("its revoke");
        drop(catalog);
        drop(txns);
        pager.close();
    });
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &pager_options(FRAMES)).expect("reopen");
        let (mut catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
        catalog
            .migrate(&pager, &txns, "module shop\nrevert 0003\n")
            .expect("the revert of the dormant grant's revoke");
        let (logged, _) = stored(&catalog, &pager, &txns);
        assert_eq!(1, logged.len(), "{logged:?}");
        assert!(logged[0].0 > 1 && logged[0].2, "back dormant: {logged:?}");
        catalog
            .migrate(&pager, &txns, "module shop\nrevert 0005\n")
            .expect("the revert of the active grant's revoke");
        let (logged, _) = stored(&catalog, &pager, &txns);
        assert_eq!(2, logged.len(), "{logged:?}");
        assert!(!logged[1].2, "back active: {logged:?}");
    });
}

#[test]
fn reverting_an_activating_regrant_makes_the_row_dormant_again() {
    let mut sim = Sim::new(152, Config::default());
    finished(&mut sim, |rt| {
        let (mut catalog, pager, txns) = dormant_ghost(&rt);
        grants::create_role(&catalog, &pager, &txns, 1, "ghost");
        catalog
            .migrate(
                &pager,
                &txns,
                "module shop\ngrant read customers to ghost\n",
            )
            .expect("the regrant");
        assert_eq!(vec![(1, false, false)], stored(&catalog, &pager, &txns).0);
        catalog
            .migrate(
                &pager,
                &txns,
                "module shop\ngrant read customers to ghost with grant option\n",
            )
            .expect("an option on the active grant");
        drop(catalog);
        drop(txns);
        pager.close();
    });
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &pager_options(FRAMES)).expect("reopen");
        let (mut catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
        catalog
            .migrate(&pager, &txns, "module shop\nrevert 0004\n")
            .expect("the revert of the option");
        assert_eq!(
            vec![(1, false, false)],
            stored(&catalog, &pager, &txns).0,
            "reverting a grant over an active entry leaves it active"
        );
        catalog
            .migrate(&pager, &txns, "module shop\nrevert 0003\n")
            .expect("the revert of the regrant");
        assert_eq!(vec![(1, false, true)], stored(&catalog, &pager, &txns).0);
    });
}

#[test]
fn a_regrant_freezing_another_field_set_leaves_the_dormant_row_dormant() {
    let mut sim = Sim::new(153, Config::default());
    finished(&mut sim, |rt| {
        let (mut catalog, pager, txns) = dormant_ghost(&rt);
        catalog
            .replay(
                &pager,
                &txns,
                "module shop\ngrant insert customers to ghost\n",
            )
            .expect("a second dormant grant");
        grants::create_role(&catalog, &pager, &txns, 1, "ghost");
        catalog
            .migrate(
                &pager,
                &txns,
                "module shop\nadd field Customer.email: Text fill \"\"\n",
            )
            .expect("the field");
        catalog
            .migrate(
                &pager,
                &txns,
                "module shop\ngrant read customers to ghost\ngrant insert customers to ghost\n",
            )
            .expect("the regrants");
        let (logged, _) = stored(&catalog, &pager, &txns);
        assert_eq!(3, logged.len(), "{logged:?}");
        assert_eq!(
            (1, false, true),
            logged[0],
            "another frozen set: {logged:?}"
        );
        assert!(!logged[1].2, "the same insert grant activates: {logged:?}");
        assert!(logged[2].0 > logged[1].0 && !logged[2].2, "{logged:?}");
    });
}

/// Returns a catalog at `SHOP` with role `ghost` missing, after `replayed` and then, once
/// `ghost` exists, `migrated`, with the rows of `catalog::grants` before `drop field
/// Customer.email` and after it.
fn merged(seed: u64, replayed: &[&str], migrated: &[&str]) -> (Vec<LogRow>, Vec<LogRow>) {
    let mut sim = Sim::new(seed, Config::default());
    let mut got = None;
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &pager_options(FRAMES)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        for text in std::iter::once(&SHOP).chain(replayed) {
            catalog
                .replay(&pager, &txns, text)
                .unwrap_or_else(|e| panic!("seed {seed}: `{text}` replays: {e:?}"));
        }
        grants::create_role(&catalog, &pager, &txns, 1, "ghost");
        for text in migrated {
            catalog
                .migrate(&pager, &txns, text)
                .unwrap_or_else(|e| panic!("seed {seed}: `{text}` applies: {e:?}"));
        }
        let before = stored(&catalog, &pager, &txns).0;
        catalog
            .migrate(&pager, &txns, "module shop\ndrop field Customer.email\n")
            .expect("the drop merges the grants");
        got = Some((before, stored(&catalog, &pager, &txns).0));
    });
    got.expect("the run finished")
}

const EMAIL: &str = "module shop\nadd field Customer.email: Text fill \"\"\n";

#[test]
fn a_prune_merge_takes_no_grant_option_from_a_dormant_entry_first() {
    let (before, after) = merged(
        154,
        &["module shop\ngrant read customers to ghost with grant option\n"],
        &[EMAIL, "module shop\ngrant read customers to ghost\n"],
    );
    assert_eq!(2, before.len(), "{before:?}");
    assert_eq!((1, true, true), before[0]);
    let active = before[1];
    assert!(!active.1 && !active.2, "{before:?}");
    assert_eq!(vec![active], after);
}

#[test]
fn a_prune_merge_takes_no_grant_option_from_a_dormant_entry_second() {
    let (before, after) = merged(
        155,
        &[
            "module shop\ngrant read customers (name) to ghost\n",
            EMAIL,
            "module shop\ngrant read customers (name, email) to ghost with grant option\n",
        ],
        &["module shop\ngrant read customers (name) to ghost\n"],
    );
    assert_eq!(2, before.len(), "{before:?}");
    assert_eq!((1, false, false), before[0], "{before:?}");
    assert!(before[1].1 && before[1].2, "{before:?}");
    assert_eq!(vec![(1, false, false)], after);
}

#[test]
fn a_prune_merge_of_two_active_entries_combines_their_grant_options() {
    let (before, after) = merged(
        156,
        &[],
        &[
            "module shop\ngrant read customers (name) to ghost with grant option\n",
            EMAIL,
            "module shop\ngrant read customers (name, email) to ghost\n",
        ],
    );
    assert_eq!(2, before.len(), "{before:?}");
    assert!(before.iter().all(|r| !r.2), "{before:?}");
    assert_eq!(vec![(before[0].0, true, false)], after);
}

#[test]
fn an_activating_regrant_gives_the_row_its_own_grant_option() {
    for (option, seed) in [("", 157), (" with grant option", 158)] {
        let mut sim = Sim::new(seed, Config::default());
        finished(&mut sim, |rt| {
            let pager = Pager::create(rt.clone(), &pager_options(FRAMES)).expect("create");
            let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
            catalog.replay(&pager, &txns, SHOP).expect("the schema");
            catalog
                .replay(
                    &pager,
                    &txns,
                    "module shop\ngrant read customers to ghost with grant option\n",
                )
                .expect("the dormant grant");
            assert_eq!(vec![(1, true, true)], stored(&catalog, &pager, &txns).0);
            grants::create_role(&catalog, &pager, &txns, 1, "ghost");
            catalog
                .migrate(
                    &pager,
                    &txns,
                    &format!("module shop\ngrant read customers to ghost{option}\n"),
                )
                .expect("the regrant");
            assert_eq!(
                vec![(1, !option.is_empty(), false)],
                stored(&catalog, &pager, &txns).0,
                "seed {seed}"
            );
        });
    }
}

/// Returns a catalog at `SHOP` with roles `web` and `bob`.
fn shop(rt: &SimRuntime) -> (Catalog, Pager<SimRuntime>, Transactions<SimRuntime>) {
    let pager = Pager::create(rt.clone(), &pager_options(FRAMES)).expect("create");
    let (mut catalog, txns) = Catalog::create(rt.clone(), &pager).expect("the catalog");
    catalog.migrate(&pager, &txns, SHOP).expect("the schema");
    grants::create_role(&catalog, &pager, &txns, 1, "web");
    grants::create_role(&catalog, &pager, &txns, 2, "bob");
    (catalog, pager, txns)
}

#[test]
fn an_operator_revoke_that_orphans_rows_granted_under_the_option_is_refused() {
    finished(&mut Sim::new(301, Config::default()), |rt| {
        let (mut catalog, pager, txns) = shop(&rt);
        let give = "module shop\ngrant read payments.Card (customer) to web with grant option\n";
        catalog.migrate(&pager, &txns, give).expect("the grant");
        grants::deployment_row(
            &catalog,
            &pager,
            &txns,
            100,
            grants::DeploymentRow {
                grantee: "bob",
                grantor: "web",
                privilege: "read",
                target: Some("shop::payments.Card (customer)"),
                grant_option: false,
                dormant: false,
            },
        );
        let revoke = "module shop\nrevoke read payments.Card (customer) from web\n";
        match catalog.migrate(&pager, &txns, revoke) {
            Err(CatalogError::Refused(why)) => {
                assert_eq!(1, why.len(), "{why:?}");
                assert!(why[0].contains("shop::payments.Card (customer)"), "{why:?}");
                assert!(why[0].contains("to `bob`"), "{why:?}");
            }
            other => panic!("the revoke applies: {other:?}"),
        }
        let keep = "module shop\nrevoke read payments.Card (customer) from web\ngrant read payments.Card (customer) to web with grant option\n";
        catalog
            .migrate(&pager, &txns, keep)
            .expect("the option stays");
        let drop = "module shop\nrevoke read payments.Card (customer) from web\ngrant read payments.Card (customer) to web\n";
        assert!(matches!(
            catalog.migrate(&pager, &txns, drop),
            Err(CatalogError::Refused(_))
        ));
    });
}

#[test]
fn an_operator_revoke_keeps_rows_a_narrower_option_does_not_cover() {
    finished(&mut Sim::new(302, Config::default()), |rt| {
        let (mut catalog, pager, txns) = shop(&rt);
        let give = "module shop\ngrant read payments.Card (customer) to web with grant option\n";
        catalog.migrate(&pager, &txns, give).expect("the grant");
        grants::deployment_row(
            &catalog,
            &pager,
            &txns,
            100,
            grants::DeploymentRow {
                grantee: "bob",
                grantor: "web",
                privilege: "read",
                target: Some("shop::payments.Invoice (customer)"),
                grant_option: false,
                dormant: false,
            },
        );
        let revoke = "module shop\nrevoke read payments.Card (customer) from web\n";
        catalog
            .migrate(&pager, &txns, revoke)
            .expect("nothing relies on it");
    });
}

/// What the log leaves in the catalog: `migration_ops`, `migration_clauses`, each migration's
/// version and hash, and `grants`, printed and sorted.
type LogRows = [Vec<String>; 4];

fn log_rows(
    catalog: &Catalog,
    pager: &Pager<SimRuntime>,
    txns: &Transactions<SimRuntime>,
) -> LogRows {
    let schema = catalog.schema();
    let read = |name: &str, keep: usize| {
        let mut rows: Vec<String> = catalog
            .committed_rows(pager, txns, values::relation(schema, name))
            .expect("the rows read")
            .into_iter()
            .map(|(_, row)| format!("{:?}", &row[..keep.min(row.len())]))
            .collect();
        rows.sort_unstable();
        rows
    };
    [
        read("migration_ops", usize::MAX),
        read("migration_clauses", usize::MAX),
        read("migrations", 2),
        read("grants", usize::MAX),
    ]
}

/// A way to apply a migration's text to a catalog.
type Apply = fn(
    &mut Catalog,
    &Pager<SimRuntime>,
    &Transactions<SimRuntime>,
    &str,
) -> Result<(), CatalogError>;

/// Returns what `log` leaves after each migration, applied with `apply` over a fresh file holding
/// roles `web`, whose log is applied after them.
fn each_version(seed: u64, log: &[&str], apply: Apply) -> Vec<LogRows> {
    let mut out = Vec::new();
    finished(&mut Sim::new(seed, Config::default()), |rt| {
        let pager = Pager::create(rt.clone(), &pager_options(FRAMES)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        grants::create_role(&catalog, &pager, &txns, 1, "web");
        for text in log {
            apply(&mut catalog, &pager, &txns, text).expect("the migration applies");
            out.push(log_rows(&catalog, &pager, &txns));
        }
    });
    out
}

/// Assert that replaying `log` to rows gives what first applying it gives, at every version.
fn replays_alike(seed: u64, log: &[&str]) -> Vec<LogRows> {
    let first = each_version(seed, log, Catalog::migrate);
    let replayed = each_version(seed + 1, log, |c, p, t, text| {
        c.replay(p, t, text).map(|_| ())
    });
    assert_eq!(first, replayed);
    first
}

#[test]
fn an_operator_revoke_acts_as_the_owner_and_replay_needs_no_revoking_role() {
    let log = [
        SHOP,
        "module shop\ngrant read payments.(Invoice | Card) (customer) to web\n",
        "module shop\nrevoke read shop::payments.(Card | Invoice) (customer) from web\n",
    ];
    let versions = replays_alike(306, &log);
    let granted = &versions[1][3];
    assert_eq!(1, granted.len(), "{granted:?}");
    assert!(granted[0].contains(r#"Text("owner")"#), "{granted:?}");
    assert!(
        granted[0].contains("(Card | Invoice) (customer)"),
        "{granted:?}"
    );
    assert_eq!(Vec::<String>::new(), versions[2][3]);
}

#[test]
fn a_deployment_revoke_leaves_a_log_row_of_the_same_text() {
    once(307, |rt| {
        let define =
            "module shop\ndefine fn shown (n: Text) -> Text = n\ngrant call fn shown to web\n";
        let db = opened(&rt, &[SHOP]);
        let authority = db.role_authority();
        authority.create_role("web").expect("role");
        drop(authority);
        drop(db);
        let db = opened(&rt, &[SHOP, define]);
        let authority = db.role_authority();
        let owner = authority.session("owner").expect("session");
        let before = by_id(&db);
        id_of(&before, &["kind: Log", "shop::shown"]);
        assert!(
            authority
                .revoke(&owner, "revoke call shop::shown from web")
                .is_err()
        );
        assert_eq!(before, by_id(&db));
    });
}

fn relied_on_by_bob(
    catalog: &Catalog,
    pager: &Pager<SimRuntime>,
    txns: &Transactions<SimRuntime>,
    target: &str,
) {
    grants::deployment_row(
        catalog,
        pager,
        txns,
        100,
        grants::DeploymentRow {
            grantee: "bob",
            grantor: "web",
            privilege: "read",
            target: Some(target),
            grant_option: false,
            dormant: false,
        },
    );
}

#[test]
fn an_operator_revoke_leaving_a_wider_option_keeps_the_rows_it_covers() {
    finished(&mut Sim::new(308, Config::default()), |rt| {
        let (mut catalog, pager, txns) = shop(&rt);
        let give = "module shop\ngrant read payments (customer) to web with grant option\ngrant read payments.Card (customer) to web with grant option\n";
        catalog.migrate(&pager, &txns, give).expect("the grants");
        relied_on_by_bob(&catalog, &pager, &txns, "shop::payments.Card (customer)");
        let revoke = "module shop\nrevoke read payments.Card (customer) from web\n";
        catalog
            .migrate(&pager, &txns, revoke)
            .expect("the wider option remains");
    });
}

#[test]
fn an_operator_revoke_is_judged_at_the_end_of_its_migration() {
    finished(&mut Sim::new(309, Config::default()), |rt| {
        let (mut catalog, pager, txns) = shop(&rt);
        let give = "module shop\ngrant read payments.Card (customer) to web with grant option\n";
        catalog.migrate(&pager, &txns, give).expect("the grant");
        relied_on_by_bob(&catalog, &pager, &txns, "shop::payments.Card (customer)");
        let widen = "module shop\nrevoke read payments.Card (customer) from web\ngrant read payments (customer) to web with grant option\n";
        catalog
            .migrate(&pager, &txns, widen)
            .expect("the migration ends with a wider option");
    });
}

#[test]
fn an_operator_revoke_of_a_row_without_an_option_orphans_nothing() {
    finished(&mut Sim::new(310, Config::default()), |rt| {
        let (mut catalog, pager, txns) = shop(&rt);
        let give = "module shop\ngrant read payments.Card (customer) to web\n";
        catalog.migrate(&pager, &txns, give).expect("the grant");
        relied_on_by_bob(&catalog, &pager, &txns, "shop::payments.Card (customer)");
        let revoke = "module shop\nrevoke read payments.Card (customer) from web\n";
        catalog
            .migrate(&pager, &txns, revoke)
            .expect("web held no option");
    });
}

#[test]
fn an_operator_grant_closing_a_cycle_of_options_is_refused() {
    finished(&mut Sim::new(311, Config::default()), |rt| {
        let (mut catalog, pager, txns) = shop(&rt);
        grants::deployment_row(
            &catalog,
            &pager,
            &txns,
            100,
            grants::DeploymentRow {
                grantee: "owner",
                grantor: "web",
                privilege: "read",
                target: Some("shop::payments.Card (customer)"),
                grant_option: true,
                dormant: false,
            },
        );
        let give = "module shop\ngrant read payments.Card (customer) to web with grant option\n";
        match catalog.migrate(&pager, &txns, give) {
            Err(CatalogError::Refused(why)) => {
                assert!(why[0].contains("cycle"), "{why:?}");
            }
            other => panic!("the grant applies: {other:?}"),
        }
        let plain = "module shop\ngrant read payments.Card (customer) to web\n";
        catalog.migrate(&pager, &txns, plain).expect("no option");
        let own = "module shop\ngrant read customers to owner with grant option\n";
        assert!(matches!(
            catalog.migrate(&pager, &txns, own),
            Err(CatalogError::Refused(_))
        ));
    });
}

#[test]
fn a_refused_empty_revoke_refuses_the_open_and_names_it() {
    once(304, |rt| {
        let log = [SHOP, "module shop\nrevoke read customers from web\n"];
        let (version, detail) = refusal(open(&rt, &log));
        assert_eq!(2, version);
        assert!(
            detail.contains("revoke read customers from web"),
            "{detail}"
        );
    });
}

#[test]
fn a_logged_privilege_list_round_trips_through_replay() {
    let log = [
        SHOP,
        "module shop\ngrant read customers (name), insert payments.(Invoice | Card), retag payments.Card -> payments.Invoice to web with grant option\nrevoke read customers (name), insert payments.(Invoice | Card) from web\n",
    ];
    let listed = |db: &Database<SimRuntime>| {
        let ops = query(
            db,
            "from o in catalog::migration_ops where o.migration == catalog::migrations.key(2)
             select match o.op
               | Grant g -> \"grant \" ++ g.privilege ++ \" \" ++ print(g.target)
               | Revoke g -> \"revoke \" ++ g.privilege ++ \" \" ++ print(g.target)
               | _ -> \"\"",
        );
        let clauses = query(
            db,
            "from c in catalog::migration_clauses where c.migration == catalog::migrations.key(2)
             select match c.body
               | Privilege p -> print(c.ordinal) ++ \" \" ++ print(c.clause) ++ \" \" ++ p.privilege ++ \" \" ++ print(p.target)
               | _ -> \"\"",
        );
        let hash = query(
            db,
            "from m in catalog::migrations where m.version == catalog::migrations.key(2) select m.hash",
        );
        (ops, clauses, hash)
    };
    once(305, |rt| {
        let db = opened(&rt, &log[..1]);
        db.role_authority().create_role("web").expect("role");
        drop(db);
        let db = opened(&rt, &log);
        let (ops, clauses, hash) = listed(&db);
        let term = |t: &str| format!("Some {{ value: Term(\\\"{t}\\\") }}");
        assert_eq!(
            vec![
                format!("\"grant read {}\"", term("shop::customers ( name )")),
                format!("\"revoke read {}\"", term("shop::customers ( name )")),
            ],
            ops
        );
        let inserted = term("shop::payments . ( shop::Invoice | shop::Card )");
        assert_eq!(
            vec![
                format!("\"1 1 insert {inserted}\""),
                format!(
                    "\"1 2 retag {}\"",
                    term("shop::payments . shop::Card -> shop::payments . shop::Invoice")
                ),
                format!("\"2 1 insert {inserted}\""),
            ],
            clauses
        );
        drop(db);
        let again = opened(&rt, &log);
        assert_eq!((ops, clauses, hash), listed(&again));
    });
    let versions = replays_alike(312, &log);
    assert_eq!(3, versions[1][1].len(), "{:?}", versions[1][1]);
}

#[test]
fn a_dormant_deployment_option_keeps_no_row_a_revoked_option_supported() {
    finished(&mut Sim::new(320, Config::default()), |rt| {
        let (mut catalog, pager, txns) = shop(&rt);
        let give = "module shop\ngrant read payments.Card (customer) to web with grant option\n";
        catalog.migrate(&pager, &txns, give).expect("the grant");
        relied_on_by_bob(&catalog, &pager, &txns, "shop::payments.Card (customer)");
        grants::deployment_row(
            &catalog,
            &pager,
            &txns,
            101,
            grants::DeploymentRow {
                grantee: "web",
                grantor: "ann",
                privilege: "read",
                target: Some("shop::payments.Card (customer)"),
                grant_option: true,
                dormant: true,
            },
        );
        let revoke = "module shop\nrevoke read payments.Card (customer) from web\n";
        assert!(
            matches!(
                catalog.migrate(&pager, &txns, revoke),
                Err(CatalogError::Refused(_))
            ),
            "a dormant option allows nothing"
        );
    });
}

#[test]
fn a_dormant_log_option_keeps_no_row_a_revoked_option_supported() {
    finished(&mut Sim::new(321, Config::default()), |rt| {
        let (mut catalog, pager, txns) = shop(&rt);
        let give = "module shop\ngrant read payments (customer) to ghost with grant option\ngrant read payments.Card (customer) to ghost with grant option\n";
        catalog
            .replay(&pager, &txns, give)
            .expect("the dormant grants");
        grants::deployment_row(
            &catalog,
            &pager,
            &txns,
            100,
            grants::DeploymentRow {
                grantee: "bob",
                grantor: "ghost",
                privilege: "read",
                target: Some("shop::payments.Card (customer)"),
                grant_option: false,
                dormant: false,
            },
        );
        let revoke = "module shop\nrevoke read payments.Card (customer) from ghost\n";
        assert!(
            matches!(
                catalog.replay(&pager, &txns, revoke),
                Err(CatalogError::Refused(_))
            ),
            "a dormant option allows nothing"
        );
    });
}
