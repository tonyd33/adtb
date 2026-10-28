mod scratch;
pub use scratch::scratch;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use campaign::web::Server;
use http_body_util::BodyExt;
use tower::ServiceExt;

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: String,
}

pub async fn request(router: &Router, method: &str, path: &str, cookie: &str, body: &str) -> Reply {
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

pub fn field<'a>(html: &'a str, before: &str, after: &str) -> &'a str {
    html.split_once(before)
        .unwrap_or_else(|| panic!("missing {before:?} in {html}"))
        .1
        .split_once(after)
        .unwrap()
        .0
}

pub fn csrf(html: &str) -> &str {
    field(html, "name=\"csrf\" value=\"", "\"")
}
pub fn location(reply: &Reply) -> &str {
    reply.headers["location"].to_str().unwrap()
}
pub fn cookie(reply: &Reply) -> &str {
    reply.headers["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
}

pub async fn login(server: &Server, router: &Router) -> (String, String) {
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

pub async fn create(router: &Router, gm: &str, token: &str) -> (String, String) {
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
