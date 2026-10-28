use std::sync::Arc;

use adtb::{CallOptions, Key};
use axum::Router;
use axum::extract::{Form, Path, Query, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{Html, Redirect};
use axum::routing::{get, post};
use serde::Deserialize;

use super::{Error, Result, State as AppState, auth, blocking, links, views};
use crate::v1_6;

pub(super) fn routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/gm", get(home))
        .route("/gm/campaigns", post(create))
        .route("/gm/campaigns/{campaign}", get(campaign))
        .route("/gm/campaigns/{campaign}/npcs", post(add_npc))
        .route("/gm/campaigns/{campaign}/entities", post(add_entity))
        .route(
            "/gm/campaigns/{campaign}/entities/{entity}",
            get(entity).post(write_entity),
        )
        .route("/gm/campaigns/{campaign}/seats", post(seat))
        .route("/gm/campaigns/{campaign}/reveal/{entity}", post(reveal))
        .route_layer(middleware::from_fn_with_state(state, auth::require_gm))
}

#[derive(Deserialize)]
struct Create {
    csrf: String,
    title: String,
    pitch: String,
    party: String,
}
#[derive(Deserialize)]
struct NewEntity {
    csrf: String,
    name: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    reference: String,
    #[serde(default)]
    agenda: String,
}
#[derive(Deserialize)]
struct Text {
    csrf: String,
    summary: String,
    body: String,
    secret: String,
}
#[derive(Deserialize)]
struct Seat {
    csrf: String,
    name: String,
    party: i64,
    character: String,
}
#[derive(Deserialize)]
struct Reveal {
    csrf: String,
    party: i64,
}
#[derive(Default, Deserialize)]
struct Search {
    #[serde(default)]
    q: String,
}

fn text(value: &str, limit: usize, required: bool) -> Result<()> {
    if value.len() > limit || (required && value.trim().is_empty()) {
        Err(Error::Http(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Enter a nonempty name and keep text within the displayed limits.",
        ))
    } else {
        Ok(())
    }
}

async fn home(State(state): State<Arc<AppState>>) -> Result<Html<String>> {
    let csrf = state.auth.csrf().to_owned();
    blocking(state, move |bundle| {
        let campaigns = bundle
            .call(v1_6::campaigns, (), &CallOptions::default())?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(views::home(&campaigns, &csrf))
    })
    .await
}

async fn create(State(state): State<Arc<AppState>>, Form(form): Form<Create>) -> Result<Redirect> {
    state.auth.check_csrf(&form.csrf)?;
    text(&form.title, 200, true)?;
    text(&form.party, 200, true)?;
    text(&form.pitch, 64000, false)?;
    let id = blocking(state, move |bundle| {
        Ok(bundle
            .call(
                v1_6::new_campaign,
                (form.title, form.pitch, form.party),
                &CallOptions::default(),
            )?
            .value
            .0)
    })
    .await?;
    Ok(Redirect::to(&format!("/gm/campaigns/{id}")))
}

async fn campaign(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(search): Query<Search>,
) -> Result<Html<String>> {
    let csrf = state.auth.csrf().to_owned();
    blocking(state, move |bundle| {
        let campaigns = bundle
            .call(v1_6::campaigns, (), &CallOptions::default())?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let title = campaigns
            .iter()
            .find_map(|c| match c {
                v1_6::CampaignsRow::Ongoing { id: key, title }
                | v1_6::CampaignsRow::Over { id: key, title, .. } => (key.0 == id).then_some(title),
            })
            .ok_or(Error::Http(StatusCode::NOT_FOUND, "Campaign not found."))?;
        let parties = bundle
            .call(v1_6::parties, (Key(id),), &CallOptions::default())?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let entities = bundle
            .call(
                v1_6::prep_entities,
                (Key(id), search.q.clone()),
                &CallOptions::default(),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(views::campaign(
            id, title, &parties, &entities, &search.q, &csrf,
        ))
    })
    .await
}

async fn add_npc(
    State(state): State<Arc<AppState>>,
    Path(campaign): Path<i64>,
    Form(mut form): Form<NewEntity>,
) -> Result<Redirect> {
    form.kind = "npc".to_owned();
    add_entity(State(state), Path(campaign), Form(form)).await
}

async fn add_entity(
    State(state): State<Arc<AppState>>,
    Path(campaign): Path<i64>,
    Form(form): Form<NewEntity>,
) -> Result<Redirect> {
    state.auth.check_csrf(&form.csrf)?;
    text(&form.name, 200, true)?;
    text(&form.summary, 1000, false)?;
    text(&form.agenda, 64000, false)?;
    let reference = if form.reference.is_empty() {
        None
    } else {
        Some(form.reference.parse::<i64>().map_err(|_| {
            Error::Http(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Choose a valid reference.",
            )
        })?)
    };
    let entity = blocking(state, move |bundle| {
        let opts = CallOptions::default();
        let c = Key(campaign);
        let key = match form.kind.as_str() {
            "npc" => {
                bundle
                    .call(
                        v1_6::add_npc,
                        (c, form.name, form.summary, reference.map(Key)),
                        &opts,
                    )?
                    .value
            }
            "region" => {
                bundle
                    .call(
                        v1_6::add_region,
                        (c, form.name, form.summary, reference.map(Key)),
                        &opts,
                    )?
                    .value
            }
            "settlement" => {
                bundle
                    .call(
                        v1_6::add_settlement,
                        (c, form.name, form.summary, reference.map(Key)),
                        &opts,
                    )?
                    .value
            }
            "site" => {
                bundle
                    .call(
                        v1_6::add_site,
                        (c, form.name, form.summary, reference.map(Key)),
                        &opts,
                    )?
                    .value
            }
            "faction" => {
                bundle
                    .call(
                        v1_6::add_faction,
                        (c, form.name, form.summary, form.agenda),
                        &opts,
                    )?
                    .value
            }
            "party" => bundle.call(v1_6::add_party, (c, form.name), &opts)?.value,
            "item" => {
                bundle
                    .call(
                        v1_6::add_item,
                        (c, form.name, form.summary, reference.map(Key)),
                        &opts,
                    )?
                    .value
            }
            _ => {
                return Err(Error::Http(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "Choose an entity kind.",
                ));
            }
        };
        Ok(key.0)
    })
    .await?;
    Ok(Redirect::to(&format!(
        "/gm/campaigns/{campaign}/entities/{entity}"
    )))
}

fn require_campaign(page: &v1_6::EntityPageRow, campaign: i64) -> Result<()> {
    if page.campaign.0 == campaign {
        Ok(())
    } else {
        Err(Error::Http(
            StatusCode::NOT_FOUND,
            "Entity not found in this campaign.",
        ))
    }
}

async fn entity(
    State(state): State<Arc<AppState>>,
    Path((campaign, entity)): Path<(i64, i64)>,
) -> Result<Html<String>> {
    let csrf = state.auth.csrf().to_owned();
    blocking(state, move |bundle| {
        let page = bundle
            .call(v1_6::entity_page, (Key(entity),), &CallOptions::default())?
            .next()
            .transpose()?
            .ok_or(Error::Http(StatusCode::NOT_FOUND, "Entity not found."))?;
        require_campaign(&page, campaign)?;
        let parties = bundle
            .call(v1_6::parties, (Key(campaign),), &CallOptions::default())?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(views::entity(campaign, entity, &page, &parties, &csrf))
    })
    .await
}

async fn write_entity(
    State(state): State<Arc<AppState>>,
    Path((campaign, entity)): Path<(i64, i64)>,
    Form(form): Form<Text>,
) -> Result<Redirect> {
    state.auth.check_csrf(&form.csrf)?;
    text(&form.summary, 1000, false)?;
    text(&form.body, 64000, false)?;
    text(&form.secret, 64000, false)?;
    blocking(state, move |bundle| {
        let page = bundle
            .call(v1_6::entity_page, (Key(entity),), &CallOptions::default())?
            .next()
            .transpose()?
            .ok_or(Error::Http(StatusCode::NOT_FOUND, "Entity not found."))?;
        require_campaign(&page, campaign)?;
        let shown = links::names(&form.body);
        let hidden = links::names(&form.secret);
        bundle.call(
            v1_6::write_entity,
            (
                Key(campaign),
                Key(entity),
                form.summary,
                form.body,
                form.secret,
                shown,
                hidden,
            ),
            &CallOptions::default(),
        )?;
        Ok(())
    })
    .await?;
    Ok(Redirect::to(&format!(
        "/gm/campaigns/{campaign}/entities/{entity}"
    )))
}

async fn seat(
    State(state): State<Arc<AppState>>,
    Path(campaign): Path<i64>,
    Form(form): Form<Seat>,
) -> Result<Html<String>> {
    state.auth.check_csrf(&form.csrf)?;
    text(&form.name, 200, true)?;
    text(&form.character, 200, true)?;
    let token = auth::token().map_err(|_| {
        Error::Http(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not generate seat token.",
        )
    })?;
    let link = format!("/join/{token}");
    blocking(state, move |bundle| {
        bundle.call(
            v1_6::seat_player,
            (
                Key(campaign),
                form.name,
                Key(form.party),
                token,
                form.character,
            ),
            &CallOptions::default(),
        )?;
        Ok(views::invite(campaign, &link))
    })
    .await
}

async fn reveal(
    State(state): State<Arc<AppState>>,
    Path((campaign, entity)): Path<(i64, i64)>,
    Form(form): Form<Reveal>,
) -> Result<Redirect> {
    state.auth.check_csrf(&form.csrf)?;
    blocking(state, move |bundle| {
        let page = bundle
            .call(v1_6::entity_page, (Key(entity),), &CallOptions::default())?
            .next()
            .transpose()?
            .ok_or(Error::Http(StatusCode::NOT_FOUND, "Entity not found."))?;
        require_campaign(&page, campaign)?;
        bundle.call(
            v1_6::reveal,
            (Key(form.party), Key(entity), None),
            &CallOptions::default(),
        )?;
        Ok(())
    })
    .await?;
    Ok(Redirect::to(&format!(
        "/gm/campaigns/{campaign}/entities/{entity}"
    )))
}
