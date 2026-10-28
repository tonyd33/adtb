mod support;

use adtb::{BundleDef, CallOptions, CreateOptions, Key, Log, OpenOptions};
use axum::Router;
use axum::http::StatusCode;
use campaign::web::Server;
use campaign::{MIGRATIONS, v1_4, v1_5};
use support::*;

fn encode(values: &[(&str, &str)]) -> String {
    values
        .iter()
        .map(|(key, value)| {
            let value: String = value
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                        char::from(b).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect();
            format!("{key}={value}")
        })
        .collect::<Vec<_>>()
        .join("&")
}

struct Prep {
    router: Router,
    gm: String,
    token: String,
    campaign: String,
    party: String,
}

impl Prep {
    async fn new(server: &Server) -> Self {
        let router = server.router();
        let (gm, token) = login(server, &router).await;
        let (campaign, party) = create(&router, &gm, &token).await;
        Self {
            router,
            gm,
            token,
            campaign,
            party,
        }
    }

    async fn add(&self, kind: &str, name: &str, reference: &str) -> String {
        let reply = request(
            &self.router,
            "POST",
            &format!("{}/entities", self.campaign),
            &self.gm,
            &encode(&[
                ("csrf", &self.token),
                ("kind", kind),
                ("name", name),
                ("summary", "Summary"),
                ("reference", reference),
                ("agenda", "SECRET-AGENDA"),
            ]),
        )
        .await;
        assert_eq!(
            reply.status,
            StatusCode::SEE_OTHER,
            "{kind} {name}: {}",
            reply.body
        );
        location(&reply).to_owned()
    }

    async fn edit(&self, entity: &str, body: &str, secret: &str) -> Reply {
        request(
            &self.router,
            "POST",
            entity,
            &self.gm,
            &encode(&[
                ("csrf", &self.token),
                ("summary", "Edited"),
                ("body", body),
                ("secret", secret),
            ]),
        )
        .await
    }

    async fn reveal(&self, entity: &str) {
        let reply = request(
            &self.router,
            "POST",
            &format!("{}/reveal/{}", self.campaign, id(entity)),
            &self.gm,
            &encode(&[("csrf", &self.token), ("party", &self.party)]),
        )
        .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
    }

    async fn player(&self) -> String {
        let invite = request(
            &self.router,
            "POST",
            &format!("{}/seats", self.campaign),
            &self.gm,
            &encode(&[
                ("csrf", &self.token),
                ("party", &self.party),
                ("name", "Player"),
                ("character", "Hero"),
            ]),
        )
        .await;
        assert_eq!(invite.status, StatusCode::OK, "{}", invite.body);
        let link = field(&invite.body, "class=\"invite\" href=\"", "\"");
        let joined = request(&self.router, "GET", link, "", "").await;
        assert_eq!(joined.status, StatusCode::SEE_OTHER);
        cookie(&joined).to_owned()
    }
}

fn id(path: &str) -> &str {
    path.rsplit('/').next().unwrap()
}
fn key(path: &str) -> Key<campaign::schema::Entities> {
    Key(id(path).parse().unwrap())
}

fn reopen(path: &std::path::Path) -> adtb::Database {
    adtb::open(
        path,
        &Log::new(MIGRATIONS),
        &OpenOptions {
            create: Some(CreateOptions::default()),
            workers: Some(2),
            registrations: vec![v1_5::BUNDLE.registration().unwrap()],
            ..OpenOptions::default()
        },
    )
    .unwrap()
}

fn scalar<T: adtb::typed::HostValue>(db: &adtb::Database, query: &str) -> T {
    let mut statements = db.query_text(query, &[], &CallOptions::default()).unwrap();
    assert_eq!(statements.len(), 1);
    let rows = statements
        .pop()
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows.len(), 1);
    T::from_value(&rows[0]).expect("query result matches host type")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prep_kinds_references_reopen_and_campaign_boundaries() {
    let path = scratch("prep-kinds");
    let server = Server::open(&path).unwrap();
    let app = Prep::new(&server).await;
    let region = app.add("region", "Realm", "").await;
    let town = app.add("settlement", "Town", id(&region)).await;
    let site = app.add("site", "Tower", id(&town)).await;
    let npc = app.add("npc", "Warden", id(&site)).await;
    let faction = app.add("faction", "Order", "").await;
    let item = app.add("item", "Seal", id(&npc)).await;
    let party = app.add("party", "New party", "").await;
    let street = app.add("site", "Straße", "").await;
    let search = request(
        &app.router,
        "GET",
        &format!("{}?q=STRASSE", app.campaign),
        &app.gm,
        "",
    )
    .await;
    assert!(
        search
            .body
            .contains(&format!("href=\"{street}\">Straße</a>"))
    );
    assert!(!search.body.contains(&format!("href=\"{npc}\">Warden</a>")));

    for entity in [&region, &town, &site, &npc, &faction, &item, &party] {
        assert_eq!(
            request(&app.router, "GET", entity, &app.gm, "")
                .await
                .status,
            StatusCode::OK
        );
    }
    let foreign = request(
        &app.router,
        "POST",
        "/gm/campaigns",
        &app.gm,
        &encode(&[
            ("csrf", &app.token),
            ("title", "Other"),
            ("pitch", ""),
            ("party", "Other party"),
        ]),
    )
    .await;
    let other = location(&foreign);
    let wrong = format!("{other}/entities/{}", id(&npc));
    assert_eq!(
        request(&app.router, "GET", &wrong, &app.gm, "")
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(app.edit(&wrong, "", "").await.status, StatusCode::NOT_FOUND);
    let bad = request(
        &app.router,
        "POST",
        &format!("{other}/entities"),
        &app.gm,
        &encode(&[
            ("csrf", &app.token),
            ("kind", "npc"),
            ("name", "Bad"),
            ("reference", id(&site)),
        ]),
    )
    .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", bad.body);
    let bad = request(
        &app.router,
        "POST",
        &format!("{}/entities", app.campaign),
        &app.gm,
        &encode(&[
            ("csrf", &app.token),
            ("kind", "npc"),
            ("name", "Wrong kind"),
            ("reference", id(&faction)),
        ]),
    )
    .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", bad.body);
    let duplicate = request(
        &app.router,
        "POST",
        &format!("{}/entities", app.campaign),
        &app.gm,
        &encode(&[("csrf", &app.token), ("kind", "region"), ("name", "Realm")]),
    )
    .await;
    assert_eq!(
        duplicate.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        duplicate.body
    );
    let home = request(&app.router, "GET", &app.campaign, &app.gm, "").await;
    assert!(!home.body.contains("SECRET-AGENDA"));
    drop(app);
    server.close().unwrap();
    let db = reopen(&path);
    let query = format!(
        "from n in entities.Npc where key n == entities.key({}) select n.location",
        id(&npc)
    );
    assert_eq!(
        scalar::<Option<Key<campaign::schema::Entities>>>(&db, &query),
        Some(key(&site))
    );
    let query = format!(
        "from x in entities.Item where key x == entities.key({}) select x.holder",
        id(&item)
    );
    assert_eq!(
        scalar::<Option<Key<campaign::schema::Entities>>>(&db, &query),
        Some(key(&npc))
    );
    let query = format!(
        "from x in entities.Settlement where key x == entities.key({}) select x.parent",
        id(&town)
    );
    assert_eq!(
        scalar::<Option<Key<campaign::schema::Entities>>>(&db, &query),
        Some(key(&region))
    );
    assert_eq!(
        scalar::<String>(&db, "from x in entities.Faction select x.agenda"),
        "SECRET-AGENDA"
    );
    db.close().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn links_visibility_atomic_replacement_and_concurrent_edits() {
    let path = scratch("prep-links");
    let server = Server::open(&path).unwrap();
    let app = Prep::new(&server).await;
    let source = app.add("npc", "Warden", "").await;
    let town = app.add("settlement", "Town", "").await;
    let vault = app.add("site", "Vault", "").await;
    let backstage = app.add("npc", "Unrevealed source", "").await;
    for n in 0..22 {
        app.add("npc", &format!("Filler {n:02}"), "").await;
    }
    let last = app.add("item", "Z-last", "").await;
    let body = "<script> [[Z-last|<b>]] [[Town]] [[Town|again]] [[Missing|alias]] [[broken";
    let secret = "SECRET-TEXT [[Town]] [[Vault|SECRET-ALIAS]]";
    assert_eq!(
        app.edit(&source, body, secret).await.status,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        app.edit(&backstage, "[[Town]]", "").await.status,
        StatusCode::SEE_OTHER
    );
    let gm = request(&app.router, "GET", &source, &app.gm, "").await;
    assert!(
        gm.body.contains(&format!("href=\"{last}\">&lt;b&gt;</a>")),
        "{}",
        gm.body
    );
    assert!(gm.body.contains("class=\"unresolved\""));
    assert_eq!(gm.body.matches(" (shared)").count(), 2);
    assert_eq!(gm.body.matches(" (secret)").count(), 2);
    let home = request(&app.router, "GET", &app.campaign, &app.gm, "").await;
    assert!(home.body.contains("Z-last"));
    assert!(!gm.body.contains("<script>"));
    for target in [&source, &town, &vault] {
        app.reveal(target).await;
    }
    let player = app.player().await;
    let shown = request(
        &app.router,
        "GET",
        &format!("/play/entities/{}", id(&source)),
        &player,
        "",
    )
    .await;
    assert_eq!(shown.status, StatusCode::OK);
    assert!(
        shown
            .body
            .contains(&format!("href=\"/play/entities/{}\">Town</a>", id(&town)))
    );
    assert!(shown.body.contains("&lt;b&gt;"));
    assert!(
        !shown
            .body
            .contains(&format!("href=\"/play/entities/{}\"", id(&last)))
    );
    for hidden in [
        "SECRET-TEXT",
        "SECRET-ALIAS",
        "Vault",
        "Z-last",
        "Missing",
        "class=\"unresolved\"",
    ] {
        assert!(
            !shown.body.contains(hidden),
            "leaked {hidden}: {}",
            shown.body
        );
    }
    let backlinks = request(
        &app.router,
        "GET",
        &format!("/play/entities/{}", id(&town)),
        &player,
        "",
    )
    .await;
    assert!(backlinks.body.contains("Warden"));
    assert!(!backlinks.body.contains("Unrevealed source"));
    let hidden = request(
        &app.router,
        "GET",
        &format!("/play/entities/{}", id(&vault)),
        &player,
        "",
    )
    .await;
    assert!(!hidden.body.contains("Warden"));
    app.reveal(&last).await;
    let shown = request(
        &app.router,
        "GET",
        &format!("/play/entities/{}", id(&source)),
        &player,
        "",
    )
    .await;
    assert!(shown.body.contains(&format!(
        "href=\"/play/entities/{}\">&lt;b&gt;</a>",
        id(&last)
    )));
    let other = request(
        &app.router,
        "POST",
        "/gm/campaigns",
        &app.gm,
        &encode(&[
            ("csrf", &app.token),
            ("title", "Other"),
            ("pitch", ""),
            ("party", "Other party"),
        ]),
    )
    .await;
    let other_campaign = location(&other).to_owned();
    let wrong = format!("{other_campaign}/entities/{}", id(&source));
    assert_eq!(app.edit(&wrong, "", "").await.status, StatusCode::NOT_FOUND);
    let gm = request(&app.router, "GET", &source, &app.gm, "").await;
    assert!(gm.body.contains("SECRET-TEXT"));
    let (a, b) = tokio::join!(
        app.edit(&source, "A [[Town]]", "A-secret"),
        app.edit(&source, "B [[Z-last]]", "B-secret [[Vault]]")
    );
    assert_eq!(a.status, StatusCode::SEE_OTHER, "{}", a.body);
    assert_eq!(b.status, StatusCode::SEE_OTHER, "{}", b.body);
    let c = Key(id(&app.campaign).parse::<i64>().unwrap());
    let other = Key(id(&other_campaign).parse::<i64>().unwrap());
    drop(app);
    server.close().unwrap();
    assert_persisted_link_edits(
        &path,
        key(&source),
        c,
        other,
        key(&town),
        key(&last),
        key(&vault),
    );
}

fn assert_persisted_link_edits(
    path: &std::path::Path,
    source_key: Key<campaign::schema::Entities>,
    c: Key<campaign::schema::Campaigns>,
    other: Key<campaign::schema::Campaigns>,
    town: Key<campaign::schema::Entities>,
    last: Key<campaign::schema::Entities>,
    vault: Key<campaign::schema::Entities>,
) {
    let db = reopen(path);
    let bundle = db.bundle(&v1_5::BUNDLE).unwrap();
    let row = bundle
        .call(
            v1_5::entity_page,
            (source_key.clone(),),
            &CallOptions::default(),
        )
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    if row.body == "A [[Town]]" {
        assert_eq!(row.secret, "A-secret");
        assert_eq!(row.mentions.len(), 1);
        assert_eq!(row.mentions[0].target, town);
        assert!(!row.mentions[0].hidden);
    } else {
        assert_eq!(row.body, "B [[Z-last]]");
        assert_eq!(row.secret, "B-secret [[Vault]]");
        assert_eq!(row.mentions.len(), 2);
        assert!(row.mentions.iter().any(|m| m.target == last && !m.hidden));
        assert!(row.mentions.iter().any(|m| m.target == vault && m.hidden));
    }
    bundle
        .call(
            v1_5::write_entity,
            (
                other,
                source_key.clone(),
                "bad".into(),
                "".into(),
                "".into(),
                vec![],
                vec![],
            ),
            &CallOptions::default(),
        )
        .unwrap();
    let unchanged = bundle
        .call(
            v1_5::entity_page,
            (source_key.clone(),),
            &CallOptions::default(),
        )
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.body, row.body);
    assert_eq!(unchanged.mentions, row.mentions);
    bundle
        .call(
            v1_5::write_entity,
            (
                c,
                source_key.clone(),
                "Cleared".into(),
                "No links".into(),
                "".into(),
                vec![],
                vec![],
            ),
            &CallOptions::default(),
        )
        .unwrap();
    let cleared = bundle
        .call(v1_5::entity_page, (source_key,), &CallOptions::default())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert!(cleared.mentions.is_empty());
    db.close().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upgrades_immutable_1_4_file_and_preserves_links() {
    let path = scratch("prep-upgrade");
    let db = adtb::open(
        &path,
        &Log::new(MIGRATIONS[..4].iter().copied()),
        &OpenOptions {
            create: Some(CreateOptions::default()),
            workers: Some(2),
            registrations: vec![v1_4::BUNDLE.registration().unwrap()],
            ..OpenOptions::default()
        },
    )
    .unwrap();
    let bundle = db.bundle(&v1_4::BUNDLE).unwrap();
    let opts = CallOptions::default();
    let c = bundle
        .call(
            v1_4::new_campaign,
            ("Old campaign".into(), "".into(), "Party".into()),
            &opts,
        )
        .unwrap()
        .value;
    let target = bundle
        .call(
            v1_4::add_npc,
            (c.clone(), "Old target".into(), "".into(), None),
            &opts,
        )
        .unwrap()
        .value;
    let source = bundle
        .call(
            v1_4::add_npc,
            (c.clone(), "Old source".into(), "".into(), None),
            &opts,
        )
        .unwrap()
        .value;
    bundle
        .call(
            v1_4::write_entity,
            (
                source.clone(),
                "".into(),
                "[[Old target]]".into(),
                "OLD-SECRET".into(),
                vec![target.clone()],
                vec![],
            ),
            &opts,
        )
        .unwrap();
    db.close().unwrap();
    let server = Server::open(&path).unwrap();
    let router = server.router();
    let (gm, _) = login(&server, &router).await;
    let source_path = format!("/gm/campaigns/{}/entities/{}", c.0, source.0);
    let page = request(&router, "GET", &source_path, &gm, "").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("OLD-SECRET"));
    assert!(page.body.contains(&format!(
        "href=\"/gm/campaigns/{}/entities/{}\">Old target</a>",
        c.0, target.0
    )));
    drop(router);
    server.close().unwrap();
}

fn migration_refuses_invalid_old_reference(kind: &str, constraint: &str) {
    let path = scratch(&format!("prep-invalid-{kind}"));
    let db = adtb::open(
        &path,
        &Log::new(MIGRATIONS[..4].iter().copied()),
        &OpenOptions {
            create: Some(CreateOptions::default()),
            workers: Some(2),
            registrations: vec![v1_4::BUNDLE.registration().unwrap()],
            ..OpenOptions::default()
        },
    )
    .unwrap();
    let bundle = db.bundle(&v1_4::BUNDLE).unwrap();
    let opts = CallOptions::default();
    let a = bundle
        .call(
            v1_4::new_campaign,
            ("A".into(), "".into(), "A party".into()),
            &opts,
        )
        .unwrap()
        .value;
    let b = bundle
        .call(
            v1_4::new_campaign,
            ("B".into(), "".into(), "B party".into()),
            &opts,
        )
        .unwrap()
        .value;
    let place = bundle
        .call(
            v1_4::add_site,
            (a.clone(), "Foreign site".into(), "".into(), None),
            &opts,
        )
        .unwrap()
        .value;
    let location = if kind == "item" { None } else { Some(place) };
    let npc = bundle
        .call(
            v1_4::add_npc,
            (b.clone(), "NPC".into(), "".into(), location),
            &opts,
        )
        .unwrap()
        .value;
    if kind == "item" {
        bundle
            .call(
                v1_4::add_item,
                (a, "Item".into(), "".into(), Some(npc)),
                &opts,
            )
            .unwrap();
    } else if kind == "fallen" {
        let party = bundle
            .call(v1_4::parties, (b.clone(),), &opts)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .id;
        let session = bundle
            .call(v1_4::plan_session, (b, party, "Session".into()), &opts)
            .unwrap()
            .value;
        bundle
            .call(v1_4::start_session, (session.clone(),), &opts)
            .unwrap();
        bundle.call(v1_4::kill, (npc, session), &opts).unwrap();
    }
    db.close().unwrap();
    let got = adtb::open(
        &path,
        &Log::new(MIGRATIONS),
        &OpenOptions {
            workers: Some(2),
            ..OpenOptions::default()
        },
    );
    let Err(adtb::OpenError::MigrationRefused { version: 5, reason }) = got else {
        panic!("expected migration5 to refuse {constraint}, got {got:?}");
    };
    let detail = match reason {
        adtb::Error::Rejected {
            reason: adtb::RejectReason::SchemaRefused { detail, .. },
        } => detail,
        other => panic!("expected schema refusal naming {constraint}, got {other:?}"),
    };
    assert!(detail.contains(constraint), "{detail}");
    let db = adtb::open(
        &path,
        &Log::new(MIGRATIONS[..4].iter().copied()),
        &OpenOptions {
            workers: Some(2),
            ..OpenOptions::default()
        },
    )
    .unwrap();
    let query = match kind {
        "npc" => "count(x for x in entities.Npc where x.location != None)",
        "fallen" => "count(x for x in entities.Fallen where x.location != None)",
        "item" => "count(x for x in entities.Item where x.holder != None)",
        _ => unreachable!(),
    };
    assert_eq!(
        scalar::<i64>(&db, query),
        1,
        "failed migration preserves offending row"
    );
    db.close().unwrap();
}

#[test]
fn migration_refuses_existing_foreign_npc_location() {
    migration_refuses_invalid_old_reference("npc", "npc_location_in_campaign");
}

#[test]
fn migration_refuses_existing_foreign_fallen_location() {
    migration_refuses_invalid_old_reference("fallen", "fallen_location_in_campaign");
}

#[test]
fn migration_refuses_existing_foreign_item_holder() {
    migration_refuses_invalid_old_reference("item", "item_holder_in_campaign");
}
