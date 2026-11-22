//! The `Log` rows of `catalog::grants`: operator revokes and grants over rows given under an
//! option, and crashes.

use crate::test_world as harness;
use crate::test_world::grants;
use adtb_testkit::process::finished;

use std::collections::BTreeMap;

use crate::catalog::codec::testing::tag;
use crate::catalog::{Catalog, Error as CatalogError, layout, values};
use crate::key::Value;
use crate::mvcc::Transactions;
use crate::storage::Pager;
use crate::verify::{self, Depth};
use adtb_lang::evolve::{Granted, Privilege};
use adtb_lang::ids::engine_ctor;
use adtb_sim::{Config, Sim, SimRuntime};

use harness::options as pager_options;

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
    let yes = values::boolean(true);
    let log = tag(schema, (r, c), "kind", engine_ctor::GRANT_LOG, Vec::new());
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

/// Returns the version, the `Log` rows and the `Deployment` ids of the catalog `files` holds
/// once it opens, after checking that it verifies clean.
fn reopened(seed: u64, files: BTreeMap<String, Vec<u8>>) -> Seen {
    let mut sim = Sim::with_files(seed, Config::default(), files);
    let mut got = None;
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &pager_options(FRAMES)).expect("reopen");
        let (mut catalog, txns) = Catalog::open(rt, &pager).expect("the catalog opens");
        let clog = txns.commit_log();
        let report = verify::verify_with(
            &pager,
            &catalog.structures().inventory(&clog),
            Depth::STRUCTURE,
        );
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
        let log = tag(
            catalog.schema(),
            (r, c),
            "kind",
            engine_ctor::GRANT_LOG,
            Vec::new(),
        );
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
                &[crate::catalog::Rewrite {
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
                Granted::read_target(schema, g.privilege(), t),
                "{t}"
            );
        }
        let retag = texts
            .iter()
            .find(|(g, _)| g.privilege() == Privilege::Retag)
            .map(|(_, t)| t.as_str());
        assert_eq!(
            Some(
                "shop::payments.(Visa | Other) -> shop::payments.(Invoice | Visa | Other) except (Visa -> Other, Other -> Visa)"
            ),
            retag
        );
    });
}
