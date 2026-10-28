use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use campaign::web::Server;
use http_body_util::BodyExt;
use tower::ServiceExt;

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

async fn request(router: &Router, method: &str, path: &str, cookie: &str, body: &str) -> Reply {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("cookie", cookie)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    Reply {
        status: parts.status,
        headers: parts.headers,
        body: String::from_utf8(body.collect().await.unwrap().to_bytes().to_vec()).unwrap(),
    }
}

fn field<'a>(html: &'a str, before: &str, after: &str) -> &'a str {
    html.split_once(before)
        .unwrap_or_else(|| panic!("missing {before:?} in {html}"))
        .1
        .split_once(after)
        .unwrap()
        .0
}

fn csrf(html: &str) -> &str {
    field(html, "name=\"csrf\" value=\"", "\"")
}
fn location(reply: &Reply) -> &str {
    reply.headers["location"].to_str().unwrap()
}
fn cookie(reply: &Reply) -> &str {
    reply.headers["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("web")
        .join(name);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("clear {dir:?}: {e}"),
        _ => {}
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("campaign.adt")
}

async fn login(server: &Server, router: &Router) -> (String, String) {
    let reply = request(router, "GET", &server.login_path(), "", "").await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(location(&reply), "/gm");
    assert!(
        reply.headers["set-cookie"]
            .to_str()
            .unwrap()
            .contains("HttpOnly; SameSite=Strict")
    );
    let gm = cookie(&reply).to_owned();
    let home = request(router, "GET", "/gm", &gm, "").await;
    assert_eq!(home.status, StatusCode::OK);
    (gm, csrf(&home.body).to_owned())
}

async fn create(router: &Router, gm: &str, token: &str) -> (String, String) {
    let reply = request(
        router,
        "POST",
        "/gm/campaigns",
        gm,
        &format!("csrf={token}&title=Ashes&pitch=A+fallen+empire&party=Lanterns"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
    let path = location(&reply).to_owned();
    let page = request(router, "GET", &path, gm, "").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    let party = field(&page.body, "option value=\"", "\"").to_owned();
    (path, party)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gm_player_flow_and_restart() {
    let path = scratch("flow");
    let server = Server::open(&path).unwrap();
    let router = server.router();
    let (gm, token) = login(&server, &router).await;
    let (campaign, party) = create(&router, &gm, &token).await;
    let created = request(
        &router,
        "POST",
        &format!("{campaign}/npcs"),
        &gm,
        &format!("csrf={token}&name=Warden+Ilsa&summary=Holds+the+pass"),
    )
    .await;
    assert_eq!(created.status, StatusCode::SEE_OTHER, "{}", created.body);
    let entity = location(&created).to_owned();
    let entity_id = entity.rsplit('/').next().unwrap();
    let edit = request(&router, "POST", &entity, &gm,
        &format!("csrf={token}&summary=Holds+the+pass&body=Shared+%3Cscript%3Ealert%281%29%3C%2Fscript%3E&secret=SECRET-PACT")).await;
    assert_eq!(edit.status, StatusCode::SEE_OTHER, "{}", edit.body);
    let gm_page = request(&router, "GET", &entity, &gm, "").await;
    assert!(gm_page.body.contains("SECRET-PACT"));
    assert!(
        gm_page
            .body
            .contains("Shared &lt;script&gt;alert(1)&lt;/script&gt;")
    );
    assert!(!gm_page.body.contains("Shared <script>"));
    let invite = request(
        &router,
        "POST",
        &format!("{campaign}/seats"),
        &gm,
        &format!("csrf={token}&name=Tony&party={party}&character=Arin"),
    )
    .await;
    assert_eq!(invite.status, StatusCode::OK, "{}", invite.body);
    let link = field(&invite.body, "class=\"invite\" href=\"", "\"");
    let join = request(&router, "GET", link, "", "").await;
    assert_eq!(join.status, StatusCode::SEE_OTHER, "{}", join.body);
    let player = cookie(&join).to_owned();
    assert_eq!(
        request(&router, "GET", "/gm", &player, "").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &router,
            "POST",
            &entity,
            &player,
            &format!("csrf={token}&summary=hacked&body=hacked&secret=hacked")
        )
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &router,
            "POST",
            &format!("/play/entities/{entity_id}"),
            &player,
            "body=hacked"
        )
        .await
        .status,
        StatusCode::METHOD_NOT_ALLOWED
    );
    let unseen = request(&router, "GET", "/play", &player, "").await;
    assert_eq!(unseen.status, StatusCode::OK);
    assert!(!unseen.body.contains("Warden Ilsa"));
    assert!(!unseen.body.contains("SECRET-PACT"));
    assert_eq!(
        request(
            &router,
            "GET",
            &format!("/play/entities/{entity_id}"),
            &player,
            ""
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    let reveal = format!("{campaign}/reveal/{entity_id}");
    for _ in 0..2 {
        let reply = request(
            &router,
            "POST",
            &reveal,
            &gm,
            &format!("csrf={token}&party={party}"),
        )
        .await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
    }
    let visible = request(&router, "GET", "/play", &player, "").await;
    assert!(visible.body.contains("Warden Ilsa"));
    let visible = request(
        &router,
        "GET",
        &format!("/play/entities/{entity_id}"),
        &player,
        "",
    )
    .await;
    assert_eq!(visible.status, StatusCode::OK);
    assert!(visible.body.contains("Shared &lt;script&gt;"));
    assert!(!visible.body.contains("SECRET-PACT"));
    assert_eq!(visible.headers["cache-control"], "no-store");
    assert_eq!(visible.headers["referrer-policy"], "no-referrer");
    assert!(
        visible.headers["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    drop(router);
    server.close().unwrap();
    let reopened = Server::open(&path).unwrap();
    let router = reopened.router();
    let persisted = request(
        &router,
        "GET",
        &format!("/play/entities/{entity_id}"),
        &player,
        "",
    )
    .await;
    assert_eq!(persisted.status, StatusCode::OK, "{}", persisted.body);
    assert!(persisted.body.contains("Shared &lt;script&gt;"));
    assert!(!persisted.body.contains("SECRET-PACT"));
    assert_eq!(
        request(&router, "GET", "/gm", &gm, "").await.status,
        StatusCode::UNAUTHORIZED
    );
    let (gm, _) = login(&reopened, &router).await;
    assert!(
        request(&router, "GET", &entity, &gm, "")
            .await
            .body
            .contains("SECRET-PACT")
    );
    drop(router);
    reopened.close().unwrap();
    assert_revoked_seat(&path, &player).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_upgrades_a_file_with_the_original_1_3_bundle() {
    use adtb::{BundleDef, CallOptions, CreateOptions, Log, OpenOptions};
    use campaign::{MIGRATIONS, v1_3};
    let path = scratch("original-bundle");
    let db = adtb::open(
        &path,
        &Log::new(MIGRATIONS),
        &OpenOptions {
            create: Some(CreateOptions::default()),
            registrations: vec![v1_3::BUNDLE.registration().unwrap()],
            workers: Some(2),
            ..OpenOptions::default()
        },
    )
    .unwrap();
    let bundle = db.bundle(&v1_3::BUNDLE).unwrap();
    let campaign = bundle
        .call(
            v1_3::new_campaign,
            (
                "Original campaign".into(),
                "Pitch".into(),
                "Original party".into(),
            ),
            &CallOptions::default(),
        )
        .unwrap()
        .value;
    db.close().unwrap();
    let server = Server::open(path).expect("new web bundle supersedes immutable original bundle");
    let router = server.router();
    let (gm, _) = login(&server, &router).await;
    let page = request(
        &router,
        "GET",
        &format!("/gm/campaigns/{}", campaign.0),
        &gm,
        "",
    )
    .await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("Original campaign"));
    assert!(page.body.contains("Original party"));
    drop(router);
    server.close().unwrap();
}

async fn assert_revoked_seat(path: &std::path::Path, cookie: &str) {
    use adtb::{BundleDef, CallOptions, Log, OpenOptions};
    use campaign::{MIGRATIONS, v1_4};
    let token = cookie.strip_prefix("seat=").unwrap().to_owned();
    let db = adtb::open(
        path,
        &Log::new(MIGRATIONS),
        &OpenOptions {
            registrations: vec![v1_4::BUNDLE.registration().unwrap()],
            workers: Some(2),
            ..OpenOptions::default()
        },
    )
    .unwrap();
    let bundle = db.bundle(&v1_4::BUNDLE).unwrap();
    let seat = bundle
        .call(v1_4::seat, (token.clone(),), &CallOptions::default())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    bundle
        .call(
            v1_4::unseat,
            (seat.player, adtb::typed::Date(20750)),
            &CallOptions::default(),
        )
        .unwrap();
    db.close().unwrap();
    let server = Server::open(path).unwrap();
    let router = server.router();
    for path in [
        "/play".to_owned(),
        "/play/entities/1".to_owned(),
        format!("/join/{token}"),
    ] {
        assert_eq!(
            request(&router, "GET", &path, cookie, "").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    drop(router);
    server.close().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authorization_invalid_input_and_concurrent_requests() {
    let server = Server::open(scratch("authorization")).unwrap();
    let router = server.router();
    assert_eq!(
        request(&router, "GET", "/gm", "", "").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &router,
            "POST",
            "/gm/campaigns",
            "seat=bogus",
            "title=attack"
        )
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&router, "GET", "/play", "seat=bogus", "")
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&router, "GET", "/join/bogus", "", "").await.status,
        StatusCode::UNAUTHORIZED
    );
    let (gm, token) = login(&server, &router).await;
    assert_eq!(
        request(&router, "GET", &server.login_path(), "", "")
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    for csrf in ["", "wrong"] {
        assert_eq!(
            request(
                &router,
                "POST",
                "/gm/campaigns",
                &gm,
                &format!("csrf={csrf}&title=attack&pitch=x&party=x")
            )
            .await
            .status,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        request(
            &router,
            "POST",
            "/gm/campaigns",
            &gm,
            &format!("csrf={token}&title=+&pitch=x&party=x")
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        request(
            &router,
            "POST",
            "/gm/campaigns",
            &gm,
            &format!("csrf={token}&title=x&pitch=x")
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (campaign, _) = create(&router, &gm, &token).await;
    let first_body = format!("csrf={token}&name=First&summary=one");
    let second_body = format!("csrf={token}&name=Second&summary=two");
    let path = format!("{campaign}/npcs");
    let (first, second, read) = tokio::join!(
        request(&router, "POST", &path, &gm, &first_body),
        request(&router, "POST", &path, &gm, &second_body),
        request(&router, "GET", &campaign, &gm, "")
    );
    assert_eq!(first.status, StatusCode::SEE_OTHER, "{}", first.body);
    assert_eq!(second.status, StatusCode::SEE_OTHER, "{}", second.body);
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    assert_ne!(location(&first), location(&second));
    let page = request(&router, "GET", &campaign, &gm, "").await;
    assert!(page.body.contains(">First</a>"));
    assert!(page.body.contains(">Second</a>"));
    let foreign = request(
        &router,
        "POST",
        &format!("{campaign}/seats"),
        &gm,
        &format!("csrf={token}&party=999999&name=Nope&character=Nope"),
    )
    .await;
    assert_eq!(
        foreign.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        foreign.body
    );
    assert_eq!(
        request(&router, "GET", "/gm/campaigns/nope", &gm, "")
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(&router, "GET", "/gm/campaigns/999999", &gm, "")
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&router, "POST", "/play", &gm, "").await.status,
        StatusCode::METHOD_NOT_ALLOWED
    );
    drop(router);
    server.close().unwrap();
}
