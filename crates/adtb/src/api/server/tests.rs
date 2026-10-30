use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use adtb_sys::os::{Backend, OsRuntime};

use crate::api::{CallOptions, CreateOptions, Database, Log, OpenError, OpenOptions, RejectReason};
use crate::catalog::Catalog;
use crate::storage::{HEADER_LEN, Pager};

use super::super::RoleAuthority;
use super::{Capability, Error, Tables, Val, print};

fn directory() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let parent = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/authority-fixtures");
    std::fs::create_dir_all(&parent).expect("create fixture parent");
    loop {
        let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("{}-{nonce}", std::process::id()));
        match std::fs::create_dir(&path) {
            Ok(()) => return path,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => panic!("create fixture directory: {error}"),
        }
    }
}

fn opened(log: &Log) -> Database {
    opened_in(&directory(), log)
}

fn opened_in(path: &std::path::Path, log: &Log) -> Database {
    let rt = OsRuntime::new(path, Backend::Pool { threads: 1 });
    Database::open_in(
        rt,
        "roles.adt",
        log,
        &OpenOptions {
            create: Some(CreateOptions::default()),
            workers: Some(1),
            pool_bytes: Some(4 * 1024 * 1024),
            ..OpenOptions::default()
        },
    )
    .expect("open authority fixture")
}

fn rows(db: &Database, text: &str) -> Vec<String> {
    let mut results = db
        .query_text(text, &[], &CallOptions::default())
        .expect("read canonical catalog rows");
    assert_eq!(1, results.len());
    let rows = results.remove(0);
    let form = rows.form().clone();
    let mut rows: Vec<_> = rows
        .map(|row| print(&row.expect("catalog row"), &form).expect("print catalog row"))
        .collect();
    rows.sort_unstable();
    rows
}

fn denied<T: std::fmt::Debug>(result: Result<T, Error>) {
    assert!(
        matches!(
            result,
            Err(Error::Rejected {
                reason: RejectReason::Denied { .. }
            })
        ),
        "{result:?}"
    );
}

fn replace_grant(authority: &RoleAuthority, field: &str, value: Val) {
    authority
        .access(true, |e, tx, id, changes| {
            let t = Tables::new(&e.schema, id);
            let grants = t.read(e, tx, t.grants)?;
            assert_eq!(1, grants.len());
            let row = &grants[0];
            let Val::Ctor(ctor, values) = row else {
                panic!("the grant fixture is a constructor");
            };
            let declared = super::layout::row(&e.schema, t.grants, *ctor);
            let mut fields: Vec<_> = declared
                .iter()
                .zip(values.iter())
                .map(|((name, _), value)| (name.as_str(), value.clone()))
                .collect();
            let entry = fields.iter_mut().find(|(name, _)| *name == field).unwrap();
            entry.1 = value.clone();
            changes.delete(t.grants, row.clone());
            changes.insert(t.grants, t.row(t.grants, "RoleGrant", fields));
            Ok(())
        })
        .expect("replace trusted grant fixture");
}

fn seed_mappings(authority: &RoleAuthority) {
    authority
        .access(true, |e, _, id, changes| {
            let t = Tables::new(&e.schema, id);
            let relation = t.peers;
            for (ordinal, role) in [(1, "web"), (3, "other"), (7, "web")] {
                let row = t.row(
                    relation,
                    "PeerMapping",
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

fn seed_credentials(authority: &RoleAuthority) {
    authority
        .access(true, |e, tx, id, changes| {
            let t = Tables::new(&e.schema, id);
            for name in ["web", "other"] {
                let role = t.role(e, tx, name)?;
                let row = t.row(
                    t.credentials,
                    "Credential",
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
    let db = opened(&Log::default());
    let authority = db.role_authority();
    authority.create_role("web").unwrap();
    let session = authority.session("web").unwrap();
    authority
        .grant_capability("web", Capability::Assert, true)
        .unwrap();
    replace_grant(&authority, "dormant", Val::bool(true));
    denied(authority.require(&session, Capability::Assert, false));
    replace_grant(&authority, "dormant", Val::bool(false));
    replace_grant(&authority, "target", Val::some(Val::text("web-1.0")));
    denied(authority.require(&session, Capability::Assert, false));
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
    denied(authority.require(&session, Capability::Assert, false));
    replace_grant(
        &authority,
        "kind",
        Val::ctor(def.ctor_index("Deployment").unwrap(), Vec::new()),
    );
    replace_grant(&authority, "privilege", Val::text("adhoc"));
    denied(authority.require(&session, Capability::Assert, false));
    authority
        .require(&session, Capability::Adhoc, true)
        .unwrap();
}

#[test]
fn dropping_a_role_removes_its_credentials() {
    let db = opened(&Log::default());
    let authority = db.role_authority();
    authority.create_role("web").unwrap();
    authority.create_role("other").unwrap();
    seed_credentials(&authority);
    let other = rows(
        &db,
        "from c in catalog::credentials, r in catalog::roles where c.role == key r and r.name == \"other\" select c",
    );
    assert_eq!(1, other.len());
    assert_eq!(rows(&db, "count(c for c in catalog::credentials)"), ["2"]);
    authority.drop_role("web").unwrap();
    assert_eq!(other, rows(&db, "from c in catalog::credentials select c"));
    authority.create_role("web").unwrap();
    assert_eq!(other, rows(&db, "from c in catalog::credentials select c"));
}

#[test]
fn dropping_a_role_removes_peer_mappings_before_name_reuse() {
    let db = opened(&Log::default());
    let authority = db.role_authority();
    authority.create_role("web").unwrap();
    authority.create_role("other").unwrap();
    seed_mappings(&authority);
    let other = rows(
        &db,
        "from m in catalog::peer_mappings where m.role == \"other\" select m",
    );
    assert_eq!(1, other.len());
    authority.drop_role("web").unwrap();
    assert_eq!(
        other,
        rows(&db, "from m in catalog::peer_mappings select m")
    );
    authority.create_role("web").unwrap();
    assert_eq!(
        other,
        rows(&db, "from m in catalog::peer_mappings select m")
    );
}

#[test]
fn a_refused_drop_leaves_peer_mappings_and_authority_unchanged() {
    let schema = "module example\nadd module example\nadd type Item = { id: Key items }\nadd relation items : Item key id\n";
    let grant = "module example\ngrant read items to web\n";
    let path = directory();
    let db = opened_in(&path, &Log::new([schema]));
    let authority = db.role_authority();
    authority.create_role("web").unwrap();
    authority.create_role("other").unwrap();
    db.close().unwrap();
    let db = opened_in(&path, &Log::new([schema, grant]));
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
    denied(authority.drop_role("web"));
    assert_eq!(before, observations.map(|query| rows(&db, query)));
    authority
        .require(&session, Capability::Assert, true)
        .unwrap();
}

#[test]
fn an_overlapping_drop_forces_a_canonical_grant_writer_to_recheck_the_role() {
    let db = opened(&Log::default());
    let authority = db.role_authority();
    authority.create_role("web").unwrap();
    let attempts = AtomicUsize::new(0);
    let (ready_tx, ready_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let writer_authority = authority.clone();
        let attempts = &attempts;
        let timeout = std::time::Duration::from_secs(5);
        let writer = scope.spawn(move || {
            writer_authority.access(true, |e, tx, id, changes| {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                let t = Tables::new(&e.schema, id);
                t.role(e, tx, "web")?;
                let grants = t.grants(e, tx)?;
                assert!(grants.is_empty());
                if attempt == 0 {
                    ready_tx.send(()).unwrap();
                    resume_rx.recv_timeout(timeout).unwrap();
                }
                let id = t.next_id(e, tx, t.grants)?;
                changes.insert(
                    t.grants,
                    t.grant_value(id, "web", Capability::Assert, "owner", true),
                );
                t.audit(
                    e,
                    tx,
                    changes,
                    "grant assert to web with grant option".into(),
                )
            })
        });
        let ready = ready_rx.recv_timeout(timeout);
        let dropped = ready.is_ok().then(|| authority.drop_role("web"));
        let released = resume_tx.send(());
        let joined = writer.join();
        assert!(
            ready.is_ok(),
            "writer reached its canonical transaction: {ready:?}"
        );
        assert!(released.is_ok(), "writer gate released: {released:?}");
        assert_eq!(Some(Ok(())), dropped);
        denied(joined.unwrap());
    });
    assert_eq!(2, attempts.load(Ordering::SeqCst));
    assert_eq!(rows(&db, "count(g for g in catalog::grants)"), ["0"]);
    assert_eq!(rows(&db, "count(a for a in catalog::audit)"), ["2"]);
}

#[test]
fn opening_an_empty_legacy_roles_table_preserves_a_full_counter_page() {
    let path = directory();
    let rt = OsRuntime::new(&path, Backend::Pool { threads: 1 });
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(1),
        pool_bytes: Some(4 * 1024 * 1024),
        ..OpenOptions::default()
    };
    let pager = Pager::create(rt.clone(), &options.storage("roles.adt")).unwrap();
    let (catalog, txns) = Catalog::open(rt.clone(), &pager).unwrap();
    let roles = Tables::new(catalog.schema(), crate::api::TxnId(1)).roles;
    assert!(
        catalog
            .committed_rows(&pager, &txns, roles)
            .unwrap()
            .is_empty()
    );
    let sequences = crate::write::full_counter_page(&pager, roles.0).unwrap();
    Catalog::naming_sequences(&pager, catalog.root()).unwrap();
    Catalog::name_sequences(&pager, catalog.root(), sequences).unwrap();
    let before = pager.read(sequences).unwrap()[HEADER_LEN..].to_vec();
    drop(catalog);
    drop(txns);
    pager.close();
    drop(pager);
    drop(rt);
    let rt = OsRuntime::new(&path, Backend::Pool { threads: 1 });
    let db = Database::open_in(rt, "roles.adt", &Log::default(), &options)
        .expect("native owner initialization must not reserve a generated counter");
    assert_eq!(
        before,
        db.engine.pager.read(sequences).unwrap()[HEADER_LEN..],
        "opening preserves every existing counter reservation"
    );
    let owner = db.role_authority().session("owner").unwrap();
    assert_eq!(1, owner.id.0);
}

fn legacy_roles(roles: &[(i64, &str)]) -> (PathBuf, OpenOptions) {
    let path = directory();
    let rt = OsRuntime::new(&path, Backend::Pool { threads: 1 });
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(1),
        pool_bytes: Some(4 * 1024 * 1024),
        ..OpenOptions::default()
    };
    let pager = Pager::create(rt.clone(), &options.storage("roles.adt")).unwrap();
    let (catalog, txns) = Catalog::open(rt, &pager).unwrap();
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
            let stored = super::layout::row(catalog.schema(), t.roles, c)
                .iter()
                .zip(fields.iter())
                .map(|((_, ty), value)| {
                    crate::exec::to_stored(&super::layout::field_type(catalog.schema(), ty), value)
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
    (path, options)
}

#[test]
fn reopening_preserves_a_preexisting_owner_identity_and_generates_past_all_roles() {
    let (path, options) = legacy_roles(&[(91, "owner"), (92, "existing")]);
    let rt = OsRuntime::new(&path, Backend::Pool { threads: 1 });
    let db = Database::open_in(rt.clone(), "roles.adt", &Log::default(), &options).unwrap();
    let before = rows(
        &db,
        "from r in catalog::roles where r.name == \"owner\" select r",
    );
    let authority = db.role_authority();
    assert_eq!(91, authority.session("owner").unwrap().id.0);
    authority.create_role("later").unwrap();
    assert!(authority.session("later").unwrap().id.0 > 92);
    db.close().unwrap();
    let db = Database::open_in(rt, "roles.adt", &Log::default(), &options).unwrap();
    assert_eq!(91, db.role_authority().session("owner").unwrap().id.0);
    assert_eq!(
        before,
        rows(
            &db,
            "from r in catalog::roles where r.name == \"owner\" select r"
        )
    );
}

#[test]
fn a_nonempty_ownerless_roles_table_is_refused_and_releases_open_resources() {
    let (path, options) = legacy_roles(&[(7, "existing")]);
    let rt = OsRuntime::new(&path, Backend::Pool { threads: 1 });
    for _ in 0..2 {
        let result = Database::open_in(rt.clone(), "roles.adt", &Log::default(), &options);
        assert!(
            matches!(result, Err(OpenError::Corrupt { detail }) if detail.contains("no schema owner"))
        );
    }
    let (pager, _) = Pager::open(rt.clone(), &options.storage("roles.adt")).unwrap();
    let (catalog, txns) = Catalog::open(rt, &pager).unwrap();
    let roles = Tables::new(catalog.schema(), crate::api::TxnId(1)).roles;
    assert_eq!(
        1,
        catalog.committed_rows(&pager, &txns, roles).unwrap().len()
    );
    pager.close();
}

#[test]
fn a_damaged_page_met_writing_a_registrations_facts_is_a_storage_failure() {
    let schema =
        "module shop\nadd module shop\nadd type C = { id: I64 }\nadd relation cs : C key id\n";
    let file = adtb_lang::parse::parse_migration(schema, adtb_lang::parse::Options::default())
        .ok()
        .expect("the migration parses");
    let source = adtb_lang::evolve::Source::default()
        .migrate(&file, schema)
        .expect("the migration applies");
    let hash: String = crate::catalog::version_hash(&source.schema().unwrap(), &source)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bundle = crate::api::Registration::bundle(format!(
        "bundle tools-0.1 pins 1 ({hash}) edition \"2026-10\"\nimport shop\n"
    ))
    .expect("the bundle parses");
    let db = opened(&Log::new([schema]));
    let e = &db.engine;
    for pid in e.deployment.roots().pages(&e.pager).unwrap() {
        e.pager.damage(pid, crate::storage::Damage::Unreadable);
    }
    let authority = db.role_authority();
    let owner = authority.session("owner").unwrap();
    let got = authority.register(&owner, &bundle);
    assert!(
        matches!(
            got,
            Err(Error::Fatal {
                reason: crate::api::FatalReason::Storage { .. }
            })
        ),
        "{got:?}"
    );
}

fn invalid(result: Result<(), Error>) -> String {
    match result {
        Err(Error::Rejected {
            reason: RejectReason::Invalid { detail },
        }) => detail,
        other => panic!("not Invalid: {other:?}"),
    }
}

#[test]
fn a_call_grant_is_revoked_by_its_target_text_and_a_family_by_any_signature() {
    let db = opened(&Log::default());
    let authority = db.role_authority();
    authority.create_role("web").unwrap();
    let owner = authority.session("owner").unwrap();
    for (row, wrong, revoke) in [
        (
            "web-2.4",
            "revoke call web-2.5 from web",
            "revoke call web-2.4 from web",
        ),
        (
            "web-2.4::q",
            "revoke call web-2.4 from web",
            "revoke call web-2.4::q from web",
        ),
        (
            "web::q at 41 (u: Key auth::users) -> Text",
            "revoke call web::q at 42 (u: Key auth::users) -> Text from web",
            "revoke call web::q from web",
        ),
        (
            "web::q at 41 (u: Key auth::users) -> Text",
            "revoke call web::r from web",
            "revoke call web::q at 41 (u: Key auth::users) -> Text from web",
        ),
    ] {
        authority
            .grant_capability("web", Capability::Assert, false)
            .unwrap();
        replace_grant(&authority, "privilege", Val::text("call"));
        replace_grant(&authority, "target", Val::some(Val::text(row)));
        assert!(
            invalid(authority.revoke(&owner, wrong)).contains(wrong),
            "{wrong}"
        );
        authority.revoke(&owner, revoke).unwrap();
        assert_eq!(
            Vec::<String>::new(),
            rows(&db, "from g in catalog::grants select g"),
            "{revoke}"
        );
    }
}

#[test]
fn a_grant_on_a_system_relation_is_revoked_by_its_text_and_widens_as_a_log_grant_does() {
    let db = opened(&Log::default());
    let authority = db.role_authority();
    authority.create_role("web").unwrap();
    let owner = authority.session("owner").unwrap();
    for (row, wrong, revoke) in [
        (
            "catalog::roles frozen (id, name)",
            "revoke read catalog::roles (id, name) from web",
            "revoke read catalog::roles from web",
        ),
        (
            "catalog::roles frozen (id, name)",
            "revoke read catalog::roles frozen (id) from web",
            "revoke read catalog::roles frozen (name, id) from web",
        ),
        (
            "catalog::roles (id, name)",
            "revoke read catalog::roles from web",
            "revoke read catalog::roles (name, id) from web",
        ),
    ] {
        authority
            .grant_capability("web", Capability::Assert, false)
            .unwrap();
        replace_grant(&authority, "privilege", Val::text("read"));
        replace_grant(&authority, "target", Val::some(Val::text(row)));
        invalid(authority.revoke(&owner, wrong));
        authority.revoke(&owner, revoke).unwrap();
    }
}

#[test]
fn a_self_given_option_is_no_remaining_option_and_is_deleted_with_its_revoke() {
    let db = opened(&Log::default());
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
    let detail = invalid(authority.revoke(&ann, "revoke monitor from web"));
    assert!(detail.contains("to `bob`"), "{detail}");
}

#[test]
fn a_revoke_from_the_schema_owner_orphans_nothing() {
    let db = opened(&Log::default());
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
}
