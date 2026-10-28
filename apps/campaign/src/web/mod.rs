mod auth;
mod gm;
mod player;
mod views;

use std::path::Path;
use std::sync::Arc;

use adtb::typed::Bundle;
use adtb::{BundleDef, CreateOptions, Database, Log, OpenOptions};
use axum::Router;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};

use crate::{MIGRATIONS, v1_4};

#[derive(Clone)]
pub struct Server {
    state: Arc<State>,
    login: String,
}

struct State {
    db: Database,
    bundle: Bundle<v1_4::Bundle>,
    auth: auth::Auth,
    workers: Arc<tokio::sync::Semaphore>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server").finish_non_exhaustive()
    }
}

#[derive(Debug)]
enum Error {
    Http(StatusCode, &'static str),
    Database(v1_4::Error),
}

type Result<T> = std::result::Result<T, Error>;

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        match self {
            Self::Http(status, message) => (status, message).into_response(),
            Self::Database(error) => {
                use std::io::Write;
                writeln!(std::io::stderr(), "campaign database call: {error:?}")
                    .unwrap_or_default();
                match error {
                    v1_4::Error::Retry { .. } => (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "Database busy; retry this action.",
                    )
                        .into_response(),
                    v1_4::Error::Rejected { .. } => (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "The database refused this action.",
                    )
                        .into_response(),
                    v1_4::Error::Fatal { .. } => {
                        (StatusCode::INTERNAL_SERVER_ERROR, "Database failure.").into_response()
                    }
                }
            }
        }
    }
}

impl From<v1_4::Error> for Error {
    fn from(error: v1_4::Error) -> Self {
        Self::Database(error)
    }
}

impl Server {
    /// Open the campaign database and prepare a one-time GM login.
    pub fn open(path: impl AsRef<Path>) -> std::result::Result<Self, String> {
        let auth = auth::Auth::new().map_err(|e| format!("generate login: {e}"))?;
        let login = auth.login_token();
        let options = OpenOptions {
            create: Some(CreateOptions::default()),
            workers: Some(2),
            registrations: vec![
                v1_4::BUNDLE
                    .registration()
                    .map_err(|e| format!("register: {e:?}"))?,
            ],
            ..OpenOptions::default()
        };
        let db = adtb::open(path, &Log::new(MIGRATIONS), &options)
            .map_err(|e| format!("open: {e:?}"))?;
        let bundle = db
            .bundle(&v1_4::BUNDLE)
            .map_err(|e| format!("bundle: {e:?}"))?;
        Ok(Self {
            state: Arc::new(State {
                db,
                bundle,
                auth,
                workers: Arc::new(tokio::sync::Semaphore::new(2)),
            }),
            login,
        })
    }

    /// Returns the one-time GM login path.
    pub fn login_path(&self) -> String {
        format!("/login/{}", self.login)
    }

    /// Returns the application router.
    pub fn router(&self) -> Router {
        Router::new()
            .merge(gm::routes(self.state.clone()))
            .merge(player::routes())
            .route("/login/{token}", axum::routing::get(auth::login))
            .route(
                "/",
                axum::routing::get(|| async { axum::response::Redirect::to("/play") }),
            )
            .with_state(self.state.clone())
            .layer(middleware::from_fn(headers))
    }

    /// Close the database after the HTTP server has stopped accepting calls.
    pub fn close(self) -> std::result::Result<(), String> {
        self.state
            .db
            .clone()
            .close()
            .map_err(|e| format!("close: {e:?}"))
    }
}

async fn blocking<T: Send + 'static>(
    state: Arc<State>,
    call: impl FnOnce(Bundle<v1_4::Bundle>) -> Result<T> + Send + 'static,
) -> Result<T> {
    let permit = state
        .workers
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| Error::Http(StatusCode::SERVICE_UNAVAILABLE, "Server is closing."))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        call(state.bundle.clone())
    })
    .await
    .unwrap_or_else(|error| {
        if error.is_panic() {
            use std::io::Write;
            writeln!(
                std::io::stderr(),
                "campaign database worker panicked: {error}"
            )
            .unwrap_or_default();
            std::process::abort();
        }
        Err(Error::Http(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database worker cancelled.",
        ))
    })
}

async fn headers(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    for (name, value) in [
        ("cache-control", "no-store"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        (
            "content-security-policy",
            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}
