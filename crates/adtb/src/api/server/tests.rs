use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adtb_lang::evolve::Privilege;
use adtb_lang::schema::{CtorRef, Schema};
use adtb_lang::types::RelId;
use adtb_sim::SimRuntime;
use adtb_testkit::process;
use adtb_testkit::refused_as;

use crate::api::bundle::Stored;
use crate::api::{Database, Log, OpenError, Registration, RejectReason};
use crate::catalog::codec::{Codec, RoleRow};
use crate::catalog::{Catalog, layout};
use crate::runtime::{Runtime as _, Threads as _};
use crate::storage::{Damage, HEADER_LEN, Pager};
use crate::test_world::api::query as rows;
use crate::test_world::load;

use super::super::RoleAuthority;
use super::{Capability, Error, Tables, Val};

type Db = Database<SimRuntime>;
type Authority = RoleAuthority<SimRuntime>;

/// Run `body` over a new database of `log` in a simulation seeded `seed`.
pub(super) fn in_database<T>(seed: u64, log: &Log, body: impl FnOnce(&SimRuntime, &Db) -> T) -> T {
    process::once(seed, |rt| {
        let db = load::opened(&rt, load::FILE, log, &load::options(1));
        body(&rt, &db)
    })
}

#[track_caller]
fn denied<T: std::fmt::Debug>(got: Result<T, Error>, detail: &str) {
    let reason = RejectReason::Denied {
        detail: detail.to_owned(),
    };
    refused_as(
        got,
        &Error::Rejected { reason },
        &format!("denied: {detail}"),
    );
}

#[track_caller]
fn refused_invalid(got: Result<(), Error>, detail: &str) {
    refused_as(got, &Error::invalid(detail), detail);
}

fn replace_grant(authority: &Authority, field: &str, value: Val) {
    authority
        .access(true, |e, tx, id, changes| {
            let t = Tables::new(&e.schema, id);
            let grants = t.read(e, tx, t.grants)?;
            assert_eq!(1, grants.len());
            let row = &grants[0];
            let Val::Ctor(ctor, values) = row else {
                panic!("the grant fixture is a constructor");
            };
            let declared = layout::row(&e.schema, t.grants, *ctor);
            let mut fields: Vec<_> = declared
                .iter()
                .zip(values.iter())
                .map(|((name, _), value)| (name.as_str(), value.clone()))
                .collect();
            let entry = fields.iter_mut().find(|(name, _)| *name == field).unwrap();
            entry.1 = value.clone();
            changes.delete(t.grants, row.clone());
            changes.insert(
                t.grants,
                fixture_row(
                    &e.schema,
                    t.grants,
                    e.schema.engine_ctors().role_grant(),
                    fields,
                ),
            );
            Ok(())
        })
        .expect("replace trusted grant fixture");
}

/// Returns the value of constructor `ctor` of `relation` whose fields `fields` gives by name.
fn fixture_row(schema: &Schema, relation: RelId, ctor: CtorRef, fields: Vec<(&str, Val)>) -> Val {
    let mut values: BTreeMap<_, _> = fields.into_iter().collect();
    let fields = layout::row(schema, relation, ctor.index())
        .iter()
        .map(|(name, _)| {
            values
                .remove(name.as_str())
                .unwrap_or_else(|| panic!("the fixture row provides {name}"))
        })
        .collect();
    assert!(
        values.is_empty(),
        "a fixture row provides exactly its declared fields"
    );
    Val::ctor(ctor.index(), fields)
}

fn ctor_of(schema: &Schema, r: RelId, name: &str) -> CtorRef {
    let td = schema.ty(schema.relation(r).elem_def);
    td.ctor_named(name)
        .expect("the catalog relation declares the constructor")
}

fn seed_mappings(authority: &Authority) {
    authority
        .access(true, |e, _, id, changes| {
            let t = Tables::new(&e.schema, id);
            let relation = t.peers;
            for (ordinal, role) in [(1, "web"), (3, "other"), (7, "web")] {
                let row = fixture_row(
                    &e.schema,
                    relation,
                    ctor_of(&e.schema, relation, "PeerMapping"),
                    vec![
                        ("listener", Val::text("local")),
                        ("ordinal", Val::Int(ordinal)),
                        ("pattern", Val::text(&format!("{role}-{ordinal}"))),
                        ("role", Val::text(role)),
                    ],
                );
                changes.insert(relation, row);
            }
            Ok(())
        })
        .expect("seed trusted peer mappings");
}

fn seed_credentials(authority: &Authority) {
    authority
        .access(true, |e, tx, id, changes| {
            let t = Tables::new(&e.schema, id);
            for name in ["web", "other"] {
                let role = t.role(e, tx, name)?;
                let row = fixture_row(
                    &e.schema,
                    t.credentials,
                    ctor_of(&e.schema, t.credentials, "Credential"),
                    vec![
                        ("id", Val::Int(role.id.0)),
                        ("role", Val::Int(role.id.0)),
                        ("salt", Val::Bytes(vec![1; 32].into())),
                        ("iterations", Val::Int(4096)),
                        ("stored_key", Val::Bytes(vec![2; 32].into())),
                        ("server_key", Val::Bytes(vec![3; 32].into())),
                    ],
                );
                changes.insert(t.credentials, row);
            }
            Ok(())
        })
        .expect("seed trusted verifiers");
}

#[test]
fn dormant_and_other_target_grants_never_authorize_a_capability() {
    in_database(401, &Log::default(), |_, db| {
        let authority = db.role_authority();
        authority.create_role("web").unwrap();
        let session = authority.session("web").unwrap();
        authority
            .grant_capability("web", Capability::Assert, true)
            .unwrap();
        replace_grant(&authority, "dormant", Val::bool(true));
        denied(
            authority.require(&session, Capability::Assert, false),
            "role `web` lacks `assert`",
        );
        replace_grant(&authority, "dormant", Val::bool(false));
        replace_grant(&authority, "target", Val::some(Val::text("web-1.0")));
        denied(
            authority.require(&session, Capability::Assert, false),
            "role `web` lacks `assert`",
        );
        replace_grant(&authority, "target", Val::none());
        let module = db.schema().module_named("catalog").unwrap();
        let def = db
            .schema()
            .ty(db.schema().module(module).types["GrantKind"]);
        replace_grant(
            &authority,
            "kind",
            Val::ctor(def.ctor_index("Log").unwrap(), Vec::new()),
        );
        denied(
            authority.require(&session, Capability::Assert, false),
            "role `web` lacks `assert`",
        );
        replace_grant(
            &authority,
            "kind",
            Val::ctor(def.ctor_index("Deployment").unwrap(), Vec::new()),
        );
        replace_grant(&authority, "privilege", Val::text("adhoc"));
        denied(
            authority.require(&session, Capability::Assert, false),
            "role `web` lacks `assert`",
        );
        authority
            .require(&session, Capability::Adhoc, true)
            .unwrap();
    });
}

#[test]
fn dropping_a_role_removes_its_credentials() {
    in_database(402, &Log::default(), |_, db| {
        let authority = db.role_authority();
        authority.create_role("web").unwrap();
        authority.create_role("other").unwrap();
        seed_credentials(&authority);
        let other = rows(
            db,
            "from c in catalog::credentials, r in catalog::roles where c.role == key r and r.name == \"other\" select c",
        );
        assert_eq!(1, other.len());
        assert_eq!(rows(db, "count(c for c in catalog::credentials)"), ["2"]);
        authority.drop_role("web").unwrap();
        assert_eq!(other, rows(db, "from c in catalog::credentials select c"));
        authority.create_role("web").unwrap();
        assert_eq!(other, rows(db, "from c in catalog::credentials select c"));
    });
}

#[test]
fn dropping_a_role_removes_peer_mappings_before_name_reuse() {
    in_database(403, &Log::default(), |_, db| {
        let authority = db.role_authority();
        authority.create_role("web").unwrap();
        authority.create_role("other").unwrap();
        seed_mappings(&authority);
        let other = rows(
            db,
            "from m in catalog::peer_mappings where m.role == \"other\" select m",
        );
        assert_eq!(1, other.len());
        authority.drop_role("web").unwrap();
        assert_eq!(other, rows(db, "from m in catalog::peer_mappings select m"));
        authority.create_role("web").unwrap();
        assert_eq!(other, rows(db, "from m in catalog::peer_mappings select m"));
    });
}

#[test]
fn a_refused_drop_leaves_peer_mappings_and_authority_unchanged() {
    let schema = "module example\nadd module example\nadd type Item = { id: Key items }\nadd relation items : Item key id\n";
    let grant = "module example\ngrant read items to web\n";
    process::once(404, |rt| {
        let db = load::opened(&rt, load::FILE, &Log::new([schema]), &load::options(1));
        let authority = db.role_authority();
        authority.create_role("web").unwrap();
        authority.create_role("other").unwrap();
        db.close().unwrap();
        let db = load::opened(
            &rt,
            load::FILE,
            &Log::new([schema, grant]),
            &load::options(1),
        );
        let authority = db.role_authority();
        authority
            .grant_capability("web", Capability::Assert, true)
            .unwrap();
        seed_mappings(&authority);
        seed_credentials(&authority);
        let observations = [
            "from m in catalog::peer_mappings select m",
            "from r in catalog::roles select r",
            "from g in catalog::grants select g",
            "from c in catalog::credentials select c",
            "from a in catalog::audit select a",
        ];
        let before = observations.map(|query| rows(&db, query));
        let session = authority.session("web").unwrap();
        denied(
            authority.drop_role("web"),
            "the role is required by the schema log",
        );
        assert_eq!(before, observations.map(|query| rows(&db, query)));
        authority
            .require(&session, Capability::Assert, true)
            .unwrap();
    });
}

/// Wait for `flag` in simulated time.
fn wait_for(rt: &SimRuntime, flag: &AtomicBool) {
    for _ in 0..1000 {
        if flag.load(Ordering::SeqCst) {
            return;
        }
        adtb_testkit::common::sleep(rt, &mut rt.io(), Duration::from_millis(5));
    }
    panic!("the flag is set within five simulated seconds");
}

#[test]
fn an_overlapping_drop_forces_a_canonical_grant_writer_to_recheck_the_role() {
    in_database(405, &Log::default(), |rt, db| {
        let authority = db.role_authority();
        authority.create_role("web").unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let ready = Arc::new(AtomicBool::new(false));
        let resume = Arc::new(AtomicBool::new(false));
        let written = Arc::new(Mutex::new(None));
        let writer = {
            let (rt, authority) = (rt.clone(), authority.clone());
            let (attempts, ready, resume) = (attempts.clone(), ready.clone(), resume.clone());
            let written = written.clone();
            rt.clone().spawn("writer", move || {
                let got = authority.access(true, |e, tx, id, changes| {
                    let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                    let t = Tables::new(&e.schema, id);
                    t.role(e, tx, "web")?;
                    let grants = t.grants(e, tx)?;
                    assert!(grants.is_empty());
                    if attempt == 0 {
                        ready.store(true, Ordering::SeqCst);
                        wait_for(&rt, &resume);
                    }
                    let id = t.next_grant_id(e, tx)?;
                    changes.insert(
                        t.grants,
                        t.grant_value(
                            id,
                            "web",
                            Privilege::Capability(Capability::Assert),
                            None,
                            "owner",
                            true,
                        ),
                    );
                    t.audit(
                        e,
                        changes,
                        "owner",
                        "grant assert to web with grant option".into(),
                    )
                });
                *written.lock().unwrap() = Some(got);
            })
        };
        wait_for(rt, &ready);
        authority.drop_role("web").unwrap();
        resume.store(true, Ordering::SeqCst);
        rt.join(writer);
        let got = written.lock().unwrap().take().expect("the writer finished");
        denied(got, "the authenticated role no longer exists");
        assert_eq!(2, attempts.load(Ordering::SeqCst));
        assert_eq!(rows(db, "count(g for g in catalog::grants)"), ["0"]);
        assert_eq!(rows(db, "count(a for a in catalog::audit)"), ["2"]);
    });
}

#[test]
fn a_rebuild_does_not_overwrite_another_change_of_the_catalog() {
    process::once(406, |rt| {
        let options = load::options(1);
        let pager = Pager::create(rt.clone(), &options.storage("changing.adt")).unwrap();
        let (catalog, txns) = Catalog::open(rt, &pager).unwrap();
        let rebuild = catalog.begin_reindexing(&pager, &txns).unwrap().unwrap();
        assert_eq!(
            None,
            catalog.begin_reindexing(&pager, &txns).unwrap().map(drop)
        );
        catalog.end_reindexing(&pager, rebuild).unwrap();
        let rebuild = catalog.begin_reindexing(&pager, &txns).unwrap().unwrap();
        catalog.end_reindexing(&pager, rebuild).unwrap();
        let naming = Catalog::naming_sequences(&pager, catalog.root()).unwrap();
        assert_eq!(
            None,
            catalog.begin_reindexing(&pager, &txns).unwrap().map(drop),
            "the change stays for the next open to free"
        );
        drop(naming);
        drop(catalog);
        drop(txns);
        pager.close();
    });
}

#[test]
fn opening_an_empty_legacy_roles_table_preserves_a_full_counter_page() {
    process::once(407, |rt| {
        let options = load::options(1);
        let pager = Pager::create(rt.clone(), &options.storage(load::FILE)).unwrap();
        let (catalog, txns) = Catalog::open(rt.clone(), &pager).unwrap();
        let roles = Codec::<RoleRow>::new(catalog.schema());
        assert!(catalog.committed(&pager, &txns, &roles).unwrap().is_empty());
        let roles = roles.relation();
        let sequences = crate::write::full_counter_page(&pager, roles.0).unwrap();
        let naming = Catalog::naming_sequences(&pager, catalog.root()).unwrap();
        Catalog::name_sequences(&pager, catalog.root(), sequences, Some(naming)).unwrap();
        let before = pager.read(sequences).unwrap()[HEADER_LEN..].to_vec();
        drop(catalog);
        drop(txns);
        pager.close();
        drop(pager);
        let db = Database::open_in(rt, load::FILE, &Log::default(), &options)
            .expect("native owner initialization must not reserve a generated counter");
        assert_eq!(
            before,
            db.engine.pager.read(sequences).unwrap()[HEADER_LEN..],
            "opening preserves every existing counter reservation"
        );
        let owner = db.role_authority().session("owner").unwrap();
        assert_eq!(1, owner.id.0);
    });
}

fn legacy_roles(rt: &SimRuntime, roles: &[(i64, &str)]) {
    let pager = Pager::create(rt.clone(), &load::options(1).storage(load::FILE)).unwrap();
    let (catalog, txns) = Catalog::open(rt.clone(), &pager).unwrap();
    let t = Tables::new(catalog.schema(), crate::api::TxnId(1));
    let inserted = roles
        .iter()
        .map(|(id, name)| {
            let row = t.role_value(&super::Role {
                id: super::RoleId(*id),
                name: (*name).to_owned(),
                created: 1234,
            });
            let Val::Ctor(c, fields) = row else {
                panic!("the role fixture is a constructor");
            };
            let stored = layout::row(catalog.schema(), t.roles, c)
                .iter()
                .zip(fields.iter())
                .map(|((_, ty), value)| {
                    crate::exec::to_stored(&layout::field_type(catalog.schema(), ty), value)
                        .unwrap()
                })
                .collect();
            (c, stored)
        })
        .collect();
    catalog
        .deploy(
            &pager,
            &txns,
            &[crate::catalog::Rewrite {
                relation: t.roles,
                deleted: Vec::new(),
                inserted,
            }],
        )
        .unwrap();
    pager.close();
}

#[test]
fn reopening_preserves_a_preexisting_owner_identity_and_generates_past_all_roles() {
    process::once(408, |rt| {
        legacy_roles(&rt, &[(91, "owner"), (92, "existing")]);
        let owner = "from r in catalog::roles where r.name == \"owner\" select r";
        let db = load::opened(&rt, load::FILE, &Log::default(), &load::options(1));
        let before = rows(&db, owner);
        let authority = db.role_authority();
        assert_eq!(91, authority.session("owner").unwrap().id.0);
        authority.create_role("later").unwrap();
        assert!(authority.session("later").unwrap().id.0 > 92);
        db.close().unwrap();
        let db = load::opened(&rt, load::FILE, &Log::default(), &load::options(1));
        assert_eq!(91, db.role_authority().session("owner").unwrap().id.0);
        assert_eq!(before, rows(&db, owner));
    });
}

#[test]
fn a_nonempty_ownerless_roles_table_is_refused_and_releases_open_resources() {
    process::once(409, |rt| {
        legacy_roles(&rt, &[(7, "existing")]);
        let options = load::options(1);
        for _ in 0..2 {
            let result = Database::open_in(rt.clone(), load::FILE, &Log::default(), &options);
            assert!(
                matches!(result, Err(OpenError::Corrupt { detail }) if detail == "a nonempty roles relation has no schema owner")
            );
        }
        let (pager, _) = Pager::open(rt.clone(), &options.storage(load::FILE)).unwrap();
        let (catalog, txns) = Catalog::open(rt, &pager).unwrap();
        let roles = Codec::<RoleRow>::new(catalog.schema());
        assert_eq!(1, catalog.committed(&pager, &txns, &roles).unwrap().len());
        pager.close();
    });
}

/// Returns the hex of the version hash of `db`'s schema.
fn version_hash(db: &Db) -> String {
    crate::catalog::version_hash(&db.engine.source)
        .expect("the log resolves")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn a_damaged_page_met_writing_a_registrations_facts_is_a_storage_failure() {
    let schema =
        "module shop\nadd module shop\nadd type C = { id: I64 }\nadd relation cs : C key id\n";
    in_database(410, &Log::new([schema]), |_, db| {
        let bundle = Registration::bundle(format!(
            "bundle tools-0.1 pins 1 ({}) edition \"2026-10\"\nimport shop\n",
            version_hash(db)
        ))
        .expect("the bundle parses");
        let e = &db.engine;
        for pid in e.deployment.roots().pages(&e.pager).unwrap() {
            e.pager.plant_damage(pid, Damage::Unreadable);
        }
        let authority = db.role_authority();
        let owner = authority.session("owner").unwrap();
        let got = authority.register(&owner, &bundle).map(drop);
        let unreadable = Error::Fatal {
            reason: crate::FatalReason::Storage {
                detail: "page 505 is unreadable".into(),
            },
        };
        assert_eq!(Err(unreadable), got);
    });
}

#[test]
fn a_self_given_option_is_no_remaining_option_and_is_deleted_with_its_revoke() {
    in_database(411, &Log::default(), |_, db| {
        let authority = db.role_authority();
        authority.create_role("web").unwrap();
        authority.create_role("bob").unwrap();
        authority.create_role("ann").unwrap();
        let owner = authority.session("owner").unwrap();
        authority
            .grant_capability("web", Capability::Monitor, true)
            .unwrap();
        replace_grant(&authority, "grantor", Val::text("web"));
        authority.revoke(&owner, "revoke monitor from web").unwrap();
        authority
            .grant_capability("web", Capability::Monitor, true)
            .unwrap();
        replace_grant(&authority, "grantor", Val::text("web"));
        authority
            .grant_capability("ann", Capability::Schema, false)
            .unwrap();
        let ann = authority.session("ann").unwrap();
        let web = authority.session("web").unwrap();
        authority
            .grant_capability_as(&ann, "web", Capability::Monitor, true)
            .unwrap();
        authority
            .grant_capability_as(&web, "bob", Capability::Monitor, false)
            .unwrap();
        refused_invalid(
            authority.revoke(&ann, "revoke monitor from web"),
            "`revoke monitor from web` takes a grant option these grants rely on: `monitor` given by `web` to `web`, `monitor` given by `web` to `bob`",
        );
    });
}

/// Set `dormant` on the one grant of `web` that `owner` gave.
fn dormant_owner_grant(authority: &Authority) {
    authority
        .access(true, |e, tx, id, changes| {
            let t = Tables::new(&e.schema, id);
            for row in t.read(e, tx, t.grants)? {
                let Val::Ctor(ctor, values) = &row else {
                    panic!("the grant fixture is a constructor");
                };
                let declared = layout::row(&e.schema, t.grants, *ctor);
                let mut fields: Vec<_> = declared
                    .iter()
                    .zip(values.iter())
                    .map(|((name, _), value)| (name.as_str(), value.clone()))
                    .collect();
                let owned = fields
                    .iter()
                    .any(|(name, value)| *name == "grantor" && *value == Val::text("owner"));
                let web = fields
                    .iter()
                    .any(|(name, value)| *name == "grantee" && *value == Val::text("web"));
                if owned && web {
                    let entry = fields.iter_mut().find(|(name, _)| *name == "dormant");
                    entry.expect("a grant row is dormant or not").1 = Val::bool(true);
                    changes.delete(t.grants, row.clone());
                    changes.insert(
                        t.grants,
                        fixture_row(
                            &e.schema,
                            t.grants,
                            e.schema.engine_ctors().role_grant(),
                            fields,
                        ),
                    );
                }
            }
            Ok(())
        })
        .expect("mark the trusted grant fixture");
}

#[test]
fn a_dormant_option_keeps_no_row_a_deployment_revoke_orphans() {
    for dormant in [false, true] {
        in_database(412, &Log::default(), |_, db| {
            let authority = db.role_authority();
            for role in ["web", "bob", "ann"] {
                authority.create_role(role).unwrap();
            }
            authority
                .grant_capability("web", Capability::Monitor, true)
                .unwrap();
            authority
                .grant_capability("ann", Capability::Schema, false)
                .unwrap();
            let ann = authority.session("ann").unwrap();
            let web = authority.session("web").unwrap();
            authority
                .grant_capability_as(&ann, "web", Capability::Monitor, true)
                .unwrap();
            authority
                .grant_capability_as(&web, "bob", Capability::Monitor, false)
                .unwrap();
            if dormant {
                dormant_owner_grant(&authority);
                refused_invalid(
                    authority.revoke(&ann, "revoke monitor from web"),
                    "`revoke monitor from web` takes a grant option these grants rely on: `monitor` given by `web` to `bob`",
                );
            } else {
                authority.revoke(&ann, "revoke monitor from web").unwrap();
            }
        });
    }
}

const SHOP: &str = "module shop\nadd module shop\nadd type Customer = { id: Key customers, name: Text }\nadd relation customers : Customer key id generate sequence\n";

#[test]
fn a_dormant_call_row_allows_nothing() {
    in_database(413, &Log::new([SHOP]), |_, db| {
        let authority = db.role_authority();
        authority.create_role("web").unwrap();
        let owner = authority.session("owner").unwrap();
        let bundle = format!(
            "bundle web-1.0 pins 1 ({}) edition \"2026-10\"\nimport shop\nquery q () -> Set Text = from c in customers select c.name\n",
            version_hash(db)
        );
        authority
            .register(&owner, &Registration::bundle(bundle).unwrap())
            .unwrap();
        authority
            .grant(&owner, "grant call web-1.0 to web")
            .unwrap();
        let web = authority.session("web").unwrap();
        authority
            .bind(&web, "web-1.0", &Default::default())
            .unwrap();
        dormant_owner_grant(&authority);
        denied(
            authority.bind(&web, "web-1.0", &Default::default()),
            "role `web` may call nothing in bundle web-1.0",
        );
    });
}

#[test]
fn a_revoke_from_the_schema_owner_orphans_nothing() {
    in_database(414, &Log::default(), |_, db| {
        let authority = db.role_authority();
        authority.create_role("web").unwrap();
        authority.create_role("bob").unwrap();
        let owner = authority.session("owner").unwrap();
        authority
            .grant_capability("web", Capability::Monitor, true)
            .unwrap();
        replace_grant(&authority, "grantor", Val::text("web"));
        replace_grant(&authority, "grantee", Val::text("owner"));
        authority
            .grant_capability("bob", Capability::Monitor, false)
            .unwrap();
        authority
            .revoke(&owner, "revoke monitor from owner")
            .unwrap();
    });
}

#[test]
fn a_role_dropped_in_the_admit_window_refuses_with_its_audit_row() {
    in_database(
        415,
        &Log::new(["module example\nadd module example\n"]),
        |_, db| {
            let authority = db.role_authority();
            authority.create_role("w").unwrap();
            let w = authority.session("w").unwrap();
            authority.drop_role("w").unwrap();
            let denied_rows = "from a in catalog::audit.Denied select a.seq";
            let service = super::service(&db.engine.schema, "w");
            assert_eq!(0, rows(db, denied_rows).len());
            denied(
                authority.admit_principal(
                    &w,
                    &service,
                    &crate::api::server::CallOrigin::AdHoc([0; 32]),
                    None,
                ),
                "the authenticated role no longer exists",
            );
            assert_eq!(1, rows(db, denied_rows).len());
        },
    );
}

/// Returns the owner's family grant once `forge` has changed the deployment state of bundle
/// `web-1.0`, which no running engine serves.
fn family_grant_over_forged_mark(seed: u64, forge: impl Fn(&mut Stored, i64)) -> Result<(), Error> {
    in_database(seed, &Log::new([SHOP]), |_, db| {
        let authority = db.role_authority();
        authority.create_role("bob").unwrap();
        let bundle = format!(
            "bundle web-1.0 pins 1 ({}) edition \"2026-10\"\nimport shop\nquery q (n: I64) -> Set Text = from c in customers select c.name\n",
            version_hash(db)
        );
        let owner = authority.session("owner").unwrap();
        authority
            .register(&owner, &Registration::bundle(bundle).unwrap())
            .unwrap();
        let e = authority.engine.upgrade().unwrap();
        e.registered.lock(&e.rt).clear();
        e.bundle_writer().change(|stored| {
            let id = stored.bundles["web-1.0"].id;
            forge(stored, id);
        });
        authority.grant(&owner, "grant call web::q to bob")
    })
}

#[test]
fn a_live_bundle_whose_signature_cannot_be_read_refuses_a_family_grant() {
    let unmarked = family_grant_over_forged_mark(416, |stored, id| {
        stored.deployed.registered.remove(&id);
    });
    refused_invalid(
        unmarked,
        "the signature of `web::q` in live bundle web-1.0 cannot be read until it is presented at an open or retired",
    );
    let garbled = family_grant_over_forged_mark(417, |stored, id| {
        stored.deployed.registered.get_mut(&id).unwrap().text = "garbled".into();
    });
    let why = Registration::bundle("garbled").unwrap_err();
    refused_invalid(
        garbled,
        &format!("the signature of `web::q` in live bundle web-1.0 cannot be read: {why}"),
    );
}

/// Run `body` over a database with bundle `cat-1.0`, reading the catalog, of role `ann`, who
/// holds `read_files` and `monitor`.
fn bundle_of_ann<T>(seed: u64, body: impl FnOnce(&Db) -> T) -> T {
    in_database(seed, &Log::new([SHOP]), |_, db| {
        let authority = db.role_authority();
        authority.create_role("ann").unwrap();
        let owner = authority.session("owner").unwrap();
        for grant in [
            "grant read catalog::constructors to ann with grant option",
            "grant read_files to ann",
            "grant monitor to ann",
        ] {
            authority.grant(&owner, grant).unwrap();
        }
        let bundle = format!(
            "bundle cat-1.0 pins 1 ({}) edition \"2026-10\"\nquery ctors () -> Set Text = from c in catalog::constructors select c.name\n",
            version_hash(db)
        );
        let ann = authority.session("ann").unwrap();
        authority
            .register(&ann, &Registration::bundle(bundle).unwrap())
            .unwrap();
        body(db)
    })
}

fn forge(db: &Db, change: impl Fn(&mut Stored, i64)) {
    let e = &db.engine;
    e.bundle_writer().change(|stored| {
        let id = stored.bundles["cat-1.0"].id;
        change(stored, id);
    });
}

#[test]
fn only_a_revoke_of_what_a_bundle_counts_looks_at_a_lacking_owner() {
    bundle_of_ann(418, |db| {
        let authority = db.role_authority();
        let owner = authority.session("owner").unwrap();
        let renewed = db.engine.deployed.read(&db.engine.rt).bundles["cat-1.0"].renewed;
        forge(db, |s, _| {
            s.bundles.get_mut("cat-1.0").unwrap().renewed = i64::MIN / 2;
        });
        authority
            .revoke(&owner, "revoke read catalog::constructors from ann")
            .unwrap();
        forge(db, |s, _| {
            s.bundles.get_mut("cat-1.0").unwrap().renewed = renewed;
        });
        refused_invalid(
            authority.revoke(&owner, "revoke read_files from ann"),
            "`revoke read_files from ann` leaves live bundles lacking what they use:\nbundle cat-1.0: ctors reads constructors.Constructor; ann lacks read with grant option\nbundle cat-1.0: ctors reads constructors.Constructor (id); ann lacks read with grant option\nbundle cat-1.0: ctors reads constructors.Constructor (name); ann lacks read with grant option",
        );
        authority.revoke(&owner, "revoke monitor from ann").unwrap();
    });
}

#[test]
fn a_bundle_of_the_grantee_that_cannot_be_read_refuses_a_counted_revoke() {
    bundle_of_ann(419, |db| {
        let authority = db.role_authority();
        let owner = authority.session("owner").unwrap();
        let e = authority.engine.upgrade().unwrap();
        e.registered.lock(&e.rt).clear();
        forge(db, |s, id| {
            s.deployed.registered.get_mut(&id).unwrap().text = "garbled".into();
        });
        let why = Registration::bundle("garbled").unwrap_err();
        refused_invalid(
            authority.revoke(&owner, "revoke read_files from ann"),
            &format!(
                "`revoke read_files from ann` leaves live bundles lacking what they use:\nbundle cat-1.0 cannot be read: {why}"
            ),
        );
        authority.revoke(&owner, "revoke monitor from ann").unwrap();
    });
}

#[test]
fn a_bundle_with_no_text_cannot_be_read_until_it_is_presented_or_retired() {
    bundle_of_ann(420, |db| {
        let authority = db.role_authority();
        let e = authority.engine.upgrade().unwrap();
        e.registered.lock(&e.rt).clear();
        forge(db, |s, id| {
            s.deployed.registered.remove(&id);
        });
        let got = authority.access(false, |e, tx, id, _| {
            let t = Tables::new(&e.schema, id);
            let grants = t.grants(e, tx)?;
            let deleted = grants
                .iter()
                .filter(|g| g.privilege == Privilege::Capability(Capability::ReadFiles))
                .map(|g| g.id)
                .collect();
            super::super::grant::Calls::new(e).refuse_lacking(
                "revoke",
                &grants,
                &deleted,
                ("owner", crate::api::OWNER_ID),
            )
        });
        refused_invalid(
            got,
            "`revoke` leaves live bundles lacking what they use:\nbundle cat-1.0 cannot be read until it is presented at an open or retired",
        );
    });
}

mod codecs {
    use adtb_lang::ids::engine_ctor as k;
    use adtb_lang::schema::Schema;

    use crate::catalog::codec::testing::{Sample, Sampled};
    use crate::key::Value;

    use super::super::{AuditRow, CredentialRow, PeerRow};

    impl Sampled for AuditRow {
        fn samples(_: &Schema) -> Vec<Sample<AuditRow>> {
            [
                k::AUDIT_LOGIN,
                k::AUDIT_GRANT,
                k::AUDIT_REGISTER,
                k::AUDIT_RETIRE,
                k::AUDIT_SCHEMA_OP,
                k::AUDIT_CALL,
                k::AUDIT_DENIED,
                k::AUDIT_COMMIT,
                k::AUDIT_TRUNCATE,
            ]
            .into_iter()
            .zip(11..)
            .map(|(id, seq)| (id, vec![("seq", Value::I64(seq))], AuditRow { seq }))
            .collect()
        }
    }

    impl Sampled for CredentialRow {
        fn samples(_: &Schema) -> Vec<Sample<CredentialRow>> {
            vec![(
                k::CREDENTIAL,
                vec![("role", Value::I64(6))],
                CredentialRow { role: 6 },
            )]
        }
    }

    impl Sampled for PeerRow {
        fn samples(_: &Schema) -> Vec<Sample<PeerRow>> {
            let want = PeerRow { role: "web".into() };
            vec![(
                k::PEER_MAPPING,
                vec![("role", Value::Text("web".into()))],
                want,
            )]
        }
    }
}
