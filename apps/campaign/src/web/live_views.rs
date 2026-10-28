use super::views::{csrf, page};
use crate::v1_6;
use axum::response::Html;
use maud::{Markup, html};

pub(super) fn index(
    campaign: i64,
    rows: &[v1_6::SessionListRow],
    parties: &[v1_6::PartiesRow],
    token: &str,
) -> Html<String> {
    page(
        "Sessions",
        html! {
            nav { a href=(format!("/gm/campaigns/{campaign}")) { "Campaign" } }
            ul { @for row in rows { li { a href=(format!("/gm/campaigns/{campaign}/sessions/{}", row.id.0)) { "Session " (row.number) ": " (row.title) } " · " (row.party) " · " (row.state) } } }
            h2 { "Plan session" }
            form method="post" { (csrf(token))
                label { "Title" input name="title" required maxlength="200"; }
                label { "Party" select name="party" required { @for party in parties { option value=(party.id.0) { (party.name) } } } }
                button { "Plan session" }
            }
        },
    )
}

pub(super) fn session(
    campaign: i64,
    id: i64,
    row: &v1_6::SessionPageRow,
    token: &str,
) -> Html<String> {
    let base = format!("/gm/campaigns/{campaign}/sessions/{id}");
    page(
        &format!("Session {}: {}", row.number, row.title),
        html! {
            nav { a href=(format!("/gm/campaigns/{campaign}/sessions")) { "Sessions" } }
            p { (row.state) }
            @if row.state != "Played" {
                form method="post" action=(format!("{base}/prep")) { (csrf(token)) label { "Prep (GM only)" textarea name="prep" maxlength="64000" { (row.prep) } } button { "Save prep" } }
            } @else {
                h2 { "Prep (GM only)" } div class="prose" { (row.prep) }
                h2 { "Log (GM only)" } div class="prose" { (row.log) }
                h2 { "Recap" } div class="prose" { (row.recap) }
            }
            @if row.state == "Planned" { form method="post" action=(format!("{base}/start")) { (csrf(token)) button { "Start session" } } }
            h2 { "Encounters" }
            ul { @for encounter in &row.encounters { li { a href=(format!("{base}/encounters/{}", encounter.id.0)) { (encounter.name) } " · " (encounter.state) } } }
            @if row.state != "Played" {
                form method="post" action=(format!("{base}/encounters")) { (csrf(token))
                    label { "Encounter name" input name="name" required maxlength="200"; }
                    label { "Setup (GM only)" textarea name="setup" maxlength="64000" {} }
                    button { "Prepare encounter" }
                }
            }
            @if row.state == "Live" {
                h2 { "End session" }
                form method="post" action=(format!("{base}/end")) { (csrf(token))
                    label { "Played on" input type="date" name="date" required; }
                    label { "Session log (GM only)" textarea name="log" maxlength="64000" {} }
                    label { "Player recap" textarea name="recap" maxlength="64000" {} }
                    button { "End session" }
                }
            }
        },
    )
}

pub(super) fn encounter(
    path: (i64, i64, i64),
    row: &v1_6::EncounterPageRow,
    roster: &[v1_6::RosterRow],
    tracker: Option<&v1_6::TrackerRow>,
    characters: &[v1_6::PrepEntitiesRow],
    token: &str,
) -> Html<String> {
    let (campaign, session, id) = path;
    let base = format!("/gm/campaigns/{campaign}/sessions/{session}/encounters/{id}");
    page(
        &row.name,
        html! {
            nav { a href=(format!("/gm/campaigns/{campaign}/sessions/{session}")) { "Session" } }
            p { (row.state) } h2 { "Setup (GM only)" } div class="prose" { (row.setup) }
            @if let Some(tracker) = tracker { (gm_tracker(&base, tracker, token)) }
            @if row.state == "Prepared" && row.session_state != "Played" {
                h2 { "Roster" }
                ul { @for member in roster { li { (member.label) } } }
                form method="post" action=(format!("{base}/cast")) { (csrf(token))
                    label { "Character" select name="who" required { @for c in characters.iter().filter(|c| c.character) { option value=(c.id.0) { (c.name) } } } }
                    label { "Cast label" input name="label" required maxlength="200"; } button { "Add cast" }
                }
                form method="post" action=(format!("{base}/extra")) { (csrf(token))
                    label { "Extra label" input name="label" required maxlength="200"; }
                    label { "Extra kind" input name="kind" required maxlength="200"; } button { "Add extra" }
                }
                @if row.session_state == "Live" && !roster.is_empty() {
                    h2 { "Initial initiative" }
                    p { "Higher rolls act first. Ties keep roster creation order." }
                    form method="post" action=(format!("{base}/start")) { (csrf(token))
                        @for member in roster { label { (member.label) " roll" input type="number" name=(format!("roll-{}", member.id.0)) required; } }
                        button { "Start encounter" }
                    }
                }
            }
            @if row.state == "Resolved" { h2 { "Aftermath" } div class="prose" { (row.aftermath) } }
        },
    )
}

fn gm_tracker(base: &str, tracker: &v1_6::TrackerRow, token: &str) -> Markup {
    let mut order: Vec<_> = tracker.order.iter().collect();
    order.sort_by_key(|t| t.slot);
    html! {
        h2 { "Round " (tracker.round) " · Turn " (tracker.turn) }
        ol { @for turn in order { li class=(if turn.slot == tracker.turn { "acting" } else { "" }) value=(turn.slot) {
            strong { (turn.label) } " · roll " (turn.roll) " · " (standing(&turn.standing))
            form method="post" action=(format!("{base}/standing")) { (csrf(token))
                input type="hidden" name="combatant" value=(turn.combatant.0);
                label { "Standing" select name="standing" { @for state in ["Ready", "Downed", "Fled", "Slain"] { option value=(state) selected[state == standing(&turn.standing)] { (state) } } } }
                label { "Tracker (GM only)" input name="tracker" value=(turn.tracker) maxlength="1000"; }
                button { "Save combatant" }
            }
        } } }
        form method="post" action=(format!("{base}/next")) { (csrf(token)) button { "Next turn" } }
        form method="post" action=(format!("{base}/end")) { (csrf(token)) label { "Aftermath" textarea name="aftermath" maxlength="64000" {} } button { "End encounter" } }
    }
}

fn standing(value: &v1_6::Standing) -> &'static str {
    match value {
        v1_6::Standing::Ready => "Ready",
        v1_6::Standing::Downed => "Downed",
        v1_6::Standing::Fled => "Fled",
        v1_6::Standing::Slain => "Slain",
    }
}

pub(super) fn now(rows: &[v1_6::PartyEncounterRow]) -> Html<String> {
    page(
        "Now",
        html! {
            nav { a href="/play" { "Party" } " · " a href="/play/recaps" { "Recaps" } }
            p { "Refresh to see the latest turn." }
            @if rows.is_empty() { p { "No encounter is running for your party." } }
            @for row in rows {
                h2 { (row.name) } p { "Round " (row.round) " · Turn " (row.turn) }
                @let order = { let mut order: Vec<_> = row.order.iter().collect(); order.sort_by_key(|t| t.slot); order };
                ol { @for t in order { li value=(t.slot) class=(if t.slot == row.turn { "acting" } else { "" }) { (t.label) " · " (standing(&t.standing)) } } }
            }
        },
    )
}

pub(super) fn recaps(rows: &[v1_6::RecapsRow]) -> Html<String> {
    page(
        "Recaps",
        html! {
            nav { a href="/play" { "Party" } " · " a href="/play/now" { "Now" } }
            @for row in rows { section { h2 { "Session " (row.number) ": " (row.title) } p { (super::dates::display(row.played_on)) } div class="prose" { (row.recap) } } }
        },
    )
}
