use std::collections::BTreeMap;
use std::sync::Arc;

use adtb::typed::Bundle;
use adtb::{CallOptions, Key};
use axum::Router;
use axum::extract::{Form, Path, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{Html, Redirect};
use axum::routing::{get, post};

use super::{Error, Result, State as AppState, auth, blocking, live_views};
use crate::v1_6;

pub(super) type Fields = BTreeMap<String, String>;

pub(super) fn routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/gm/campaigns/{campaign}/sessions", get(index).post(plan))
        .route("/gm/campaigns/{campaign}/sessions/{session}", get(page))
        .route(
            "/gm/campaigns/{campaign}/sessions/{session}/{action}",
            post(action),
        )
        .merge(super::combat::routes())
        .route_layer(middleware::from_fn_with_state(state, auth::require_gm))
}

pub(super) fn field<'a>(form: &'a Fields, name: &str) -> Result<&'a str> {
    form.get(name).map(String::as_str).ok_or(Error::Http(
        StatusCode::UNPROCESSABLE_ENTITY,
        "Missing form field.",
    ))
}

pub(super) fn text(form: &Fields, name: &str, limit: usize, required: bool) -> Result<String> {
    let value = field(form, name)?;
    if value.len() > limit || (required && value.trim().is_empty()) {
        return Err(Error::Http(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Enter text within the displayed limits.",
        ));
    }
    Ok(value.to_owned())
}

pub(super) fn integer(form: &Fields, name: &str) -> Result<i64> {
    field(form, name)?
        .parse()
        .map_err(|_| Error::Http(StatusCode::UNPROCESSABLE_ENTITY, "Enter a valid integer."))
}

pub(super) fn require(condition: bool) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::Http(
            StatusCode::CONFLICT,
            "This action is unavailable in the current state.",
        ))
    }
}

pub(super) fn session(
    bundle: &Bundle<v1_6::Bundle>,
    campaign: i64,
    id: i64,
) -> Result<v1_6::SessionPageRow> {
    bundle
        .call(v1_6::session_page, (Key(id),), &CallOptions::default())?
        .next()
        .transpose()?
        .filter(|row| row.campaign.0 == campaign)
        .ok_or(Error::Http(
            StatusCode::NOT_FOUND,
            "Session not found in this campaign.",
        ))
}

async fn index(
    State(state): State<Arc<AppState>>,
    Path(campaign): Path<i64>,
) -> Result<Html<String>> {
    let token = state.auth.csrf().to_owned();
    blocking(state, move |bundle| {
        let opts = CallOptions::default();
        let rows = bundle
            .call(v1_6::session_list, (Key(campaign),), &opts)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let parties = bundle
            .call(v1_6::parties, (Key(campaign),), &opts)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(live_views::index(campaign, &rows, &parties, &token))
    })
    .await
}

async fn plan(
    State(state): State<Arc<AppState>>,
    Path(campaign): Path<i64>,
    Form(form): Form<Fields>,
) -> Result<Redirect> {
    state.auth.check_csrf(field(&form, "csrf")?)?;
    let title = text(&form, "title", 200, true)?;
    let party = integer(&form, "party")?;
    let id = blocking(state, move |bundle| {
        Ok(bundle
            .call(
                v1_6::plan_session,
                (Key(campaign), Key(party), title),
                &CallOptions::default(),
            )?
            .value
            .0)
    })
    .await?;
    Ok(Redirect::to(&format!(
        "/gm/campaigns/{campaign}/sessions/{id}"
    )))
}

async fn page(
    State(state): State<Arc<AppState>>,
    Path((campaign, id)): Path<(i64, i64)>,
) -> Result<Html<String>> {
    let token = state.auth.csrf().to_owned();
    blocking(state, move |bundle| {
        let row = session(&bundle, campaign, id)?;
        Ok(live_views::session(campaign, id, &row, &token))
    })
    .await
}

async fn action(
    State(state): State<Arc<AppState>>,
    Path((campaign, id, action)): Path<(i64, i64, String)>,
    Form(form): Form<Fields>,
) -> Result<Redirect> {
    state.auth.check_csrf(field(&form, "csrf")?)?;
    blocking(state, move |bundle| {
        let row = session(&bundle, campaign, id)?;
        let opts = CallOptions::default();
        match action.as_str() {
            "prep" => {
                require(row.state != "Played")?;
                bundle.call(
                    v1_6::write_session_prep,
                    (Key(id), text(&form, "prep", 64000, false)?),
                    &opts,
                )?;
            }
            "start" => {
                require(row.state == "Planned")?;
                bundle.call(v1_6::start_session, (Key(id),), &opts)?;
            }
            "encounters" => {
                require(row.state != "Played")?;
                let key = bundle
                    .call(
                        v1_6::add_encounter,
                        (
                            Key(id),
                            text(&form, "name", 200, true)?,
                            text(&form, "setup", 64000, false)?,
                        ),
                        &opts,
                    )?
                    .value;
                return Ok(format!(
                    "/gm/campaigns/{campaign}/sessions/{id}/encounters/{}",
                    key.0
                ));
            }
            "end" => {
                require(row.state == "Live")?;
                bundle.call(
                    v1_6::end_session,
                    (
                        Key(id),
                        super::dates::parse(field(&form, "date")?)?,
                        text(&form, "log", 64000, false)?,
                        text(&form, "recap", 64000, false)?,
                    ),
                    &opts,
                )?;
            }
            _ => return Err(Error::Http(StatusCode::NOT_FOUND, "Action not found.")),
        }
        Ok(format!("/gm/campaigns/{campaign}/sessions/{id}"))
    })
    .await
    .map(|path| Redirect::to(&path))
}
