use super::{Error, Result, State as AppState, blocking, live, live_views};
use crate::v1_6;
use adtb::typed::Bundle;
use adtb::{CallOptions, Key};
use axum::Router;
use axum::extract::{Form, Path, State};
use axum::http::StatusCode;
use axum::response::{Html, Redirect};
use axum::routing::{get, post};
use live::{Fields, field, integer, require, text};
use std::sync::Arc;

type EncounterPath = (i64, i64, i64);
type ActionPath = (i64, i64, i64, String);

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/gm/campaigns/{campaign}/sessions/{session}/encounters/{encounter}",
            get(page),
        )
        .route(
            "/gm/campaigns/{campaign}/sessions/{session}/encounters/{encounter}/{action}",
            post(action),
        )
}

fn encounter(
    bundle: &Bundle<v1_6::Bundle>,
    campaign: i64,
    session: i64,
    id: i64,
) -> Result<v1_6::EncounterPageRow> {
    bundle
        .call(v1_6::encounter_page, (Key(id),), &CallOptions::default())?
        .next()
        .transpose()?
        .filter(|row| row.campaign.0 == campaign && row.session.0 == session)
        .ok_or(Error::Http(
            StatusCode::NOT_FOUND,
            "Encounter not found in this session.",
        ))
}

async fn page(
    State(state): State<Arc<AppState>>,
    Path((campaign, session, id)): Path<EncounterPath>,
) -> Result<Html<String>> {
    let token = state.auth.csrf().to_owned();
    blocking(state, move |bundle| {
        let row = encounter(&bundle, campaign, session, id)?;
        let opts = CallOptions::default();
        let mut roster = bundle
            .call(v1_6::roster, (Key(id),), &opts)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        roster.sort_by_key(|r| r.id.0);
        let tracker = bundle
            .call(v1_6::tracker, (Key(id),), &opts)?
            .next()
            .transpose()?;
        let characters = bundle
            .call(v1_6::prep_entities, (Key(campaign), String::new()), &opts)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(live_views::encounter(
            (campaign, session, id),
            &row,
            &roster,
            tracker.as_ref(),
            &characters,
            &token,
        ))
    })
    .await
}

fn rolls(form: &Fields, roster: &[v1_6::RosterRow]) -> Result<Vec<v1_6::StartEncounterRollsArg>> {
    require(!roster.is_empty())?;
    if form.len() != roster.len() + 1 {
        return Err(Error::Http(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Enter one roll for every combatant.",
        ));
    }
    let mut order = roster
        .iter()
        .map(|r| Ok((r.id.clone(), integer(form, &format!("roll-{}", r.id.0))?)))
        .collect::<Result<Vec<_>>>()?;
    order.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.0.cmp(&b.0.0)));
    order
        .into_iter()
        .enumerate()
        .map(|(slot, (combatant, roll))| {
            Ok(v1_6::StartEncounterRollsArg {
                combatant,
                roll,
                slot: i64::try_from(slot + 1).map_err(|_| {
                    Error::Http(StatusCode::UNPROCESSABLE_ENTITY, "Too many combatants.")
                })?,
            })
        })
        .collect()
}

fn standing(value: &str) -> Result<v1_6::Standing> {
    match value {
        "Ready" => Ok(v1_6::Standing::Ready),
        "Downed" => Ok(v1_6::Standing::Downed),
        "Fled" => Ok(v1_6::Standing::Fled),
        "Slain" => Ok(v1_6::Standing::Slain),
        _ => Err(Error::Http(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Choose a standing.",
        )),
    }
}

async fn action(
    State(state): State<Arc<AppState>>,
    Path((campaign, session, id, action)): Path<ActionPath>,
    Form(form): Form<Fields>,
) -> Result<Redirect> {
    state.auth.check_csrf(field(&form, "csrf")?)?;
    blocking(state, move |bundle| {
        let row = encounter(&bundle, campaign, session, id)?;
        let opts = CallOptions::default();
        let mut roster = bundle
            .call(v1_6::roster, (Key(id),), &opts)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        roster.sort_by_key(|r| r.id.0);
        match action.as_str() {
            "cast" => {
                require(row.state == "Prepared" && row.session_state != "Played")?;
                bundle.call(
                    v1_6::add_cast,
                    (
                        Key(id),
                        Key(integer(&form, "who")?),
                        text(&form, "label", 200, true)?,
                    ),
                    &opts,
                )?;
            }
            "extra" => {
                require(row.state == "Prepared" && row.session_state != "Played")?;
                bundle.call(
                    v1_6::add_extra,
                    (
                        Key(id),
                        text(&form, "label", 200, true)?,
                        text(&form, "kind", 200, true)?,
                    ),
                    &opts,
                )?;
            }
            "start" => {
                require(row.state == "Prepared" && row.session_state == "Live")?;
                bundle.call(
                    v1_6::start_encounter,
                    (Key(id), rolls(&form, &roster)?),
                    &opts,
                )?;
            }
            "next" => {
                require(row.state == "Underway")?;
                bundle.call(v1_6::next_turn, (Key(id),), &opts)?;
            }
            "standing" => {
                require(row.state == "Underway")?;
                let combatant = integer(&form, "combatant")?;
                if !roster.iter().any(|r| r.id.0 == combatant) {
                    return Err(Error::Http(
                        StatusCode::NOT_FOUND,
                        "Combatant not found in this encounter.",
                    ));
                }
                bundle.call(
                    v1_6::set_standing,
                    (
                        Key(combatant),
                        standing(field(&form, "standing")?)?,
                        text(&form, "tracker", 1000, false)?,
                    ),
                    &opts,
                )?;
            }
            "end" => {
                require(row.state == "Underway")?;
                bundle.call(
                    v1_6::end_encounter,
                    (Key(id), text(&form, "aftermath", 64000, false)?),
                    &opts,
                )?;
            }
            _ => return Err(Error::Http(StatusCode::NOT_FOUND, "Action not found.")),
        }
        Ok(())
    })
    .await?;
    Ok(Redirect::to(&format!(
        "/gm/campaigns/{campaign}/sessions/{session}/encounters/{id}"
    )))
}
