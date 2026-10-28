use std::sync::Arc;

use adtb::{CallOptions, Key};
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;

use super::{Error, Result, State as AppState, auth, blocking, views};
use crate::v1_4;

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/join/{token}", get(join))
        .route("/play", get(home))
        .route("/play/entities/{entity}", get(entity))
}

async fn join(State(state): State<Arc<AppState>>, Path(token): Path<String>) -> Result<Response> {
    let given = token.clone();
    blocking(state, move |bundle| {
        bundle
            .call(v1_4::seat, (given,), &CallOptions::default())?
            .next()
            .transpose()?
            .ok_or(Error::Http(
                StatusCode::UNAUTHORIZED,
                "This seat link is invalid or revoked.",
            ))?;
        Ok(())
    })
    .await?;
    Ok((
        [("set-cookie", auth::set_cookie("seat", &token))],
        Redirect::to("/play"),
    )
        .into_response())
}

fn seat_token(headers: &HeaderMap) -> Result<String> {
    auth::cookie(headers, "seat")
        .map(str::to_owned)
        .ok_or(Error::Http(
            StatusCode::UNAUTHORIZED,
            "Join using a seat link from your GM.",
        ))
}

async fn home(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Result<Html<String>> {
    let token = seat_token(&headers)?;
    blocking(state, move |bundle| {
        let seat = bundle
            .call(v1_4::seat, (token,), &CallOptions::default())?
            .next()
            .transpose()?
            .ok_or(Error::Http(
                StatusCode::UNAUTHORIZED,
                "This seat has been revoked.",
            ))?;
        let entities = bundle
            .call(v1_4::known_entities, (seat.party,), &CallOptions::default())?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(views::player(&seat.name, &entities))
    })
    .await
}

async fn entity(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(entity): Path<i64>,
) -> Result<Html<String>> {
    let token = seat_token(&headers)?;
    blocking(state, move |bundle| {
        let seat = bundle
            .call(v1_4::seat, (token,), &CallOptions::default())?
            .next()
            .transpose()?
            .ok_or(Error::Http(
                StatusCode::UNAUTHORIZED,
                "This seat has been revoked.",
            ))?;
        let page = bundle
            .call(
                v1_4::known_entity,
                (seat.party, Key(entity)),
                &CallOptions::default(),
            )?
            .next()
            .transpose()?
            .ok_or(Error::Http(StatusCode::NOT_FOUND, "Entity not found."))?;
        Ok(views::player_entity(&page))
    })
    .await
}
