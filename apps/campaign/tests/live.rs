#[path = "live/atomic.rs"]
mod atomic;
mod support;

use adtb::{BundleDef, CallOptions, CreateOptions, Key, Log, OpenOptions};
use axum::Router;
use axum::http::StatusCode;
use campaign::web::Server;
use campaign::{MIGRATIONS, v1_5, v1_6};
use support::*;

struct App {
    router: Router,
    gm: String,
    csrf: String,
    campaign: String,
    party: String,
}

impl App {
    async fn new(server: &Server) -> Self {
        let router = server.router();
        let (gm, csrf) = login(server, &router).await;
        let (campaign, party) = create(&router, &gm, &csrf).await;
        Self {
            router,
            gm,
            csrf,
            campaign,
            party,
        }
    }
    async fn post(&self, path: &str, values: &str) -> Reply {
        request(
            &self.router,
            "POST",
            path,
            &self.gm,
            &format!("csrf={}&{values}", self.csrf),
        )
        .await
    }
    async fn redirected(&self, path: &str, values: &str) -> String {
        let r = self.post(path, values).await;
        assert_eq!(r.status, StatusCode::SEE_OTHER, "{path}: {}", r.body);
        location(&r).to_owned()
    }
    async fn session(&self, title: &str) -> String {
        self.redirected(
            &format!("{}/sessions", self.campaign),
            &format!("party={}&title={title}", self.party),
        )
        .await
    }
    async fn encounter(&self, session: &str) -> (String, Vec<String>) {
        let e = self
            .redirected(
                &format!("{session}/encounters"),
                "name=Bandits&setup=SECRET-SETUP",
            )
            .await;
        let npc = self
            .redirected(
                &format!("{}/npcs", self.campaign),
                "name=Warden&summary=SECRET-NPC",
            )
            .await;
        self.redirected(
            &format!("{e}/cast"),
            &format!("who={}&label=Bandit", id(&npc)),
        )
        .await;
        self.redirected(&format!("{e}/extra"), "label=Wolf&kind=Beast")
            .await;
        let r = request(&self.router, "GET", &e, &self.gm, "").await;
        let ids = r
            .body
            .split("name=\"roll-")
            .skip(1)
            .map(|s| s.split('"').next().unwrap().to_owned())
            .collect();
        (e, ids)
    }
    async fn player(&self) -> String {
        let r = self
            .post(
                &format!("{}/seats", self.campaign),
                &format!("name=Player&party={}&character=Hero", self.party),
            )
            .await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
        let link = field(&r.body, "class=\"invite\" href=\"", "\"");
        let r = request(&self.router, "GET", link, "", "").await;
        assert_eq!(r.status, StatusCode::SEE_OTHER);
        cookie(&r).to_owned()
    }
}

fn open(path: &std::path::Path) -> adtb::Database {
    adtb::open(
        path,
        &Log::new(MIGRATIONS),
        &OpenOptions {
            registrations: vec![v1_6::BUNDLE.registration().unwrap()],
            workers: Some(2),
            ..OpenOptions::default()
        },
    )
    .unwrap()
}
fn id(path: &str) -> i64 {
    path.rsplit('/').next().unwrap().parse().unwrap()
}
fn tracker(db: &adtb::Database, encounter: &str) -> v1_6::TrackerRow {
    db.bundle(&v1_6::BUNDLE)
        .unwrap()
        .call(
            v1_6::tracker,
            (Key(id(encounter)),),
            &CallOptions::default(),
        )
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_flow_concurrent_turns_reopen_and_player_secrets() {
    let path = scratch("live-flow");
    let server = Server::open(&path).unwrap();
    let app = App::new(&server).await;
    let player = app.player().await;
    let session = app.session("First").await;
    app.redirected(&format!("{session}/prep"), "prep=SECRET-PREP")
        .await;
    app.redirected(&format!("{session}/start"), "").await;
    let (encounter, ids) = app.encounter(&session).await;
    assert_eq!(ids.len(), 2);
    assert_eq!(
        app.post(&format!("{encounter}/cast"), "who=999999&label=Bad")
            .await
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.post(&format!("{encounter}/start"), "").await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    app.redirected(
        &format!("{encounter}/start"),
        &format!("roll-{}=12&roll-{}=12", ids[0], ids[1]),
    )
    .await;
    app.redirected(
        &format!("{encounter}/standing"),
        &format!(
            "combatant={}&standing=Downed&tracker=SECRET-TRACKER",
            ids[0]
        ),
    )
    .await;
    let next = format!("{encounter}/next");
    let (a, b, read) = tokio::join!(
        app.post(&next, ""),
        app.post(&next, ""),
        request(&app.router, "GET", "/play/now", &player, "")
    );
    assert_eq!(a.status, StatusCode::SEE_OTHER, "{}", a.body);
    assert_eq!(b.status, StatusCode::SEE_OTHER, "{}", b.body);
    assert_eq!(read.status, StatusCode::OK);
    for secret in [
        "SECRET-SETUP",
        "SECRET-PREP",
        "SECRET-TRACKER",
        "SECRET-NPC",
    ] {
        assert!(!read.body.contains(secret));
    }
    assert!(read.body.contains("Bandit") && read.body.contains("Downed"));
    assert_eq!(
        request(&app.router, "POST", &next, &player, "")
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app.router, "GET", &encounter, &player, "")
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app.router, "POST", &next, &app.gm, "csrf=wrong")
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    drop(app);
    server.close().unwrap();
    let db = open(&path);
    let t = tracker(&db, &encounter);
    assert_eq!((t.round, t.turn), (2, 1));
    let first = t.order.iter().find(|t| t.slot == 1).unwrap();
    assert_eq!(first.combatant.0.to_string(), ids[0]);
    assert_eq!(first.tracker, "SECRET-TRACKER");
    db.close().unwrap();
    let server = Server::open(&path).unwrap();
    let router = server.router();
    let (gm, token) = login(&server, &router).await;
    let r = request(
        &router,
        "POST",
        &format!("{session}/end"),
        &gm,
        &format!("csrf={token}&date=2026-02-30&log=x&recap=y"),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    let r = request(
        &router,
        "POST",
        &format!("{session}/end"),
        &gm,
        &format!("csrf={token}&date=2026-10-28&log=SECRET-LOG&recap=Victory"),
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    let recap = request(&router, "GET", "/play/recaps", &player, "").await;
    assert!(recap.body.contains("Victory"));
    assert_eq!(recap.status, StatusCode::OK);
    assert!(!recap.body.contains("SECRET-LOG"));
    let now = request(&router, "GET", "/play/now", &player, "").await;
    assert!(now.body.contains("No encounter is running"));
    drop(router);
    server.close().unwrap();
    let db = open(&path);
    let bundle = db.bundle(&v1_6::BUNDLE).unwrap();
    assert!(
        bundle
            .call(
                v1_6::tracker,
                (Key(id(&encounter)),),
                &CallOptions::default()
            )
            .unwrap()
            .next()
            .is_none()
    );
    let s = bundle
        .call(
            v1_6::session_page,
            (Key(id(&session)),),
            &CallOptions::default(),
        )
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        (s.state.as_str(), s.log.as_str(), s.recap.as_str()),
        ("Played", "SECRET-LOG", "Victory")
    );
    db.close().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_planning_starts_and_cross_path_refusals() {
    let path = scratch("live-boundaries");
    let server = Server::open(&path).unwrap();
    let app = App::new(&server).await;
    let (a, b) = tokio::join!(app.session("One"), app.session("Two"));
    let start_a = format!("{a}/start");
    let start_b = format!("{b}/start");
    let (ra, rb) = tokio::join!(app.post(&start_a, ""), app.post(&start_b, ""));
    assert_eq!(
        [ra.status, rb.status]
            .iter()
            .filter(|&&s| s == StatusCode::SEE_OTHER)
            .count(),
        1
    );
    assert_eq!(
        [ra.status, rb.status]
            .iter()
            .filter(|&&s| s == StatusCode::UNPROCESSABLE_ENTITY)
            .count(),
        1
    );
    let live = if ra.status == StatusCode::SEE_OTHER {
        &a
    } else {
        &b
    };
    let planned = if ra.status == StatusCode::SEE_OTHER {
        &b
    } else {
        &a
    };
    let (encounter, ids) = app.encounter(live).await;
    let wrong = encounter.replacen(live, planned, 1);
    assert_eq!(
        request(&app.router, "GET", &wrong, &app.gm, "")
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.post(&format!("{wrong}/next"), "").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.post(
            &format!("{encounter}/start"),
            &format!("roll-{}=oops&roll-{}=1", ids[0], ids[1])
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    app.redirected(
        &format!("{encounter}/start"),
        &format!("roll-{}=1&roll-{}=20", ids[0], ids[1]),
    )
    .await;
    assert_eq!(
        app.post(
            &format!("{encounter}/standing"),
            "combatant=999999&standing=Ready&tracker=x"
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    app.redirected(&format!("{encounter}/end"), "aftermath=Fled")
        .await;
    let foreign = app
        .redirected("/gm/campaigns", "title=Other&pitch=&party=Other")
        .await;
    let wrong_session = a.replacen(&app.campaign, &foreign, 1);
    assert_eq!(
        app.post(&format!("{encounter}/cast"), "who=999999&label=Bad")
            .await
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&app.router, "GET", &wrong_session, &app.gm, "")
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.post(
            &format!("{foreign}/sessions"),
            &format!("party={}&title=Bad", app.party)
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    drop(app);
    server.close().unwrap();
    let db = open(&path);
    let bundle = db.bundle(&v1_6::BUNDLE).unwrap();
    let ca = bundle
        .call(v1_6::session_page, (Key(id(&a)),), &CallOptions::default())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let cb = bundle
        .call(v1_6::session_page, (Key(id(&b)),), &CallOptions::default())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_ne!(ca.number, cb.number);
    assert!(tracker_missing(&bundle, &encounter));
    atomic::concurrent_roster_addition_and_start_cannot_both_commit(&bundle, Key(id(live)));
    atomic::stale_rosters_and_ended_lifecycles_are_rejected_inside_registered_calls(
        &bundle,
        Key(id(live)),
    );
    db.close().unwrap();
}

fn tracker_missing(bundle: &adtb::typed::Bundle<v1_6::Bundle>, e: &str) -> bool {
    bundle
        .call(v1_6::tracker, (Key(id(e)),), &CallOptions::default())
        .unwrap()
        .next()
        .is_none()
}

#[test]
fn prior_prep_bundle_file_upgrades_without_changing_schema() {
    let path = scratch("live-upgrade");
    let db = adtb::open(
        &path,
        &Log::new(MIGRATIONS),
        &OpenOptions {
            create: Some(CreateOptions::default()),
            registrations: vec![v1_5::BUNDLE.registration().unwrap()],
            ..OpenOptions::default()
        },
    )
    .unwrap();
    let bundle = db.bundle(&v1_5::BUNDLE).unwrap();
    let campaign = bundle
        .call(
            v1_5::new_campaign,
            ("Old".into(), "".into(), "Party".into()),
            &CallOptions::default(),
        )
        .unwrap()
        .value;
    db.close().unwrap();
    let server = Server::open(&path).unwrap();
    server.close().unwrap();
    let db = open(&path);
    assert_eq!(
        db.bundle(&v1_6::BUNDLE)
            .unwrap()
            .call(v1_6::campaigns, (), &CallOptions::default())
            .unwrap()
            .next()
            .unwrap()
            .unwrap(),
        v1_6::CampaignsRow::Ongoing {
            id: Key(campaign.0),
            title: "Old".into()
        }
    );
    db.close().unwrap();
}

#[tokio::test]
async fn revoked_seats_cannot_read_live_or_recap_pages() {
    let path = scratch("live-revocation");
    let server = Server::open(&path).unwrap();
    let app = App::new(&server).await;
    let player = app.player().await;
    assert_eq!(
        request(&app.router, "GET", "/play/now", &player, "")
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        request(&app.router, "POST", "/play/now", &player, "")
            .await
            .status,
        StatusCode::METHOD_NOT_ALLOWED
    );
    drop(app);
    server.close().unwrap();
    let db = open(&path);
    let bundle = db.bundle(&v1_6::BUNDLE).unwrap();
    let token = player.strip_prefix("seat=").unwrap().to_owned();
    let seat = bundle
        .call(v1_6::seat, (token,), &CallOptions::default())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    bundle
        .call(
            v1_6::unseat,
            (seat.player, adtb::typed::Date(20754)),
            &CallOptions::default(),
        )
        .unwrap();
    db.close().unwrap();
    let server = Server::open(&path).unwrap();
    let router = server.router();
    for path in ["/play/now", "/play/recaps"] {
        assert_eq!(
            request(&router, "GET", path, &player, "").await.status,
            StatusCode::UNAUTHORIZED,
            "{path}"
        );
    }
    drop(router);
    server.close().unwrap();
}
