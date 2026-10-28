use std::io::Read;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};

use super::{Error, Result, State as AppState};

pub(super) struct Auth {
    login: Mutex<Option<String>>,
    session: String,
    csrf: String,
}

impl Auth {
    pub(super) fn new() -> std::io::Result<Self> {
        Ok(Self {
            login: Mutex::new(Some(token()?)),
            session: token()?,
            csrf: token()?,
        })
    }

    pub(super) fn login_token(&self) -> String {
        self.login
            .lock()
            .expect("login lock is not poisoned")
            .as_ref()
            .expect("login is unused at startup")
            .clone()
    }

    pub(super) fn csrf(&self) -> &str {
        &self.csrf
    }

    pub(super) fn check_csrf(&self, given: &str) -> Result<()> {
        if given == self.csrf {
            Ok(())
        } else {
            Err(Error::Http(StatusCode::FORBIDDEN, "Invalid form token."))
        }
    }
}

pub(super) fn token() -> std::io::Result<String> {
    let mut bytes = [0_u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub(super) fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all("cookie")
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|h| h.split(';'))
        .filter_map(|p| p.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

pub(super) fn set_cookie(name: &str, token: &str) -> String {
    format!("{name}={token}; Path=/; HttpOnly; SameSite=Strict")
}

pub(super) async fn login(
    State(state): State<Arc<AppState>>,
    Path(given): Path<String>,
) -> Result<Response> {
    let mut login = state.auth.login.lock().expect("login lock is not poisoned");
    if login.as_deref() != Some(&given) {
        return Err(Error::Http(
            StatusCode::UNAUTHORIZED,
            "Invalid or already used login link.",
        ));
    }
    *login = None;
    Ok((
        [("set-cookie", set_cookie("gm", &state.auth.session))],
        Redirect::to("/gm"),
    )
        .into_response())
}

pub(super) async fn require_gm(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
    next: Next,
) -> Result<Response> {
    if cookie(request.headers(), "gm") != Some(&state.auth.session) {
        return Err(Error::Http(
            StatusCode::UNAUTHORIZED,
            "Sign in using the GM link printed by the server.",
        ));
    }
    Ok(next.run(request).await)
}
