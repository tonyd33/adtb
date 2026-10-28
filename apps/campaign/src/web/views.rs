use axum::response::Html;
use maud::{DOCTYPE, Markup, html};

use crate::v1_4;

fn page(title: &str, content: Markup) -> Html<String> {
    Html(
        html! {
            (DOCTYPE)
            html lang="en" {
                head {
                    meta charset="utf-8";
                    meta name="viewport" content="width=device-width, initial-scale=1";
                    title { (title) " · Campaign" }
                    style { (include_str!("style.css")) }
                }
                body { main { h1 { (title) } (content) } }
            }
        }
        .into_string(),
    )
}

fn csrf(token: &str) -> Markup {
    html! { input type="hidden" name="csrf" value=(token); }
}

fn parties(rows: &[v1_4::PartiesRow]) -> Markup {
    html! { label { "Party" select name="party" required {
        @for party in rows { option value=(party.id.0) { (party.name) } }
    } } }
}

pub(super) fn home(rows: &[v1_4::CampaignsRow], token: &str) -> Html<String> {
    page(
        "Campaigns",
        html! {
            ul { @for row in rows {
                @let (id, title) = match row {
                    v1_4::CampaignsRow::Ongoing { id, title } | v1_4::CampaignsRow::Over { id, title, .. } => (id, title),
                };
                li { a href=(format!("/gm/campaigns/{}", id.0)) { (title) } }
            } }
            section { h2 { "New campaign" }
                form method="post" action="/gm/campaigns" {
                    (csrf(token))
                    label { "Title" input name="title" required maxlength="200"; }
                    label { "Pitch" textarea name="pitch" maxlength="64000" {} }
                    label { "First party" input name="party" required maxlength="200"; }
                    button { "Create campaign" }
                }
            }
        },
    )
}

pub(super) fn campaign(
    id: i64,
    title: &str,
    groups: &[v1_4::PartiesRow],
    entities: &[v1_4::LookupRow],
    query: &str,
    token: &str,
) -> Html<String> {
    page(
        title,
        html! {
            nav { a href="/gm" { "All campaigns" } }
            form method="get" { label { "Find an entity" input name="q" value=(query); } button { "Search" } }
            p { "Up to 20 matches. Search by name to find more." }
            ul { @for row in entities { li {
                a href=(format!("/gm/campaigns/{id}/entities/{}", row.id.0)) { (row.name) }
                " · " (row.kind) " · " (row.summary)
            } } }
            section { h2 { "Create NPC" } p { "Create a name and summary, then edit shared and secret text on the NPC page." }
                form method="post" action=(format!("/gm/campaigns/{id}/npcs")) {
                    (csrf(token))
                    label { "Name" input name="name" required maxlength="200"; }
                    label { "Summary" input name="summary" maxlength="1000"; }
                    button { "Create NPC and edit text" }
                }
            }
            section { h2 { "Invite player" }
                form method="post" action=(format!("/gm/campaigns/{id}/seats")) {
                    (csrf(token)) (parties(groups))
                    label { "Player name" input name="name" required maxlength="200"; }
                    label { "Character name" input name="character" required maxlength="200"; }
                    button { "Create seat link" }
                }
            }
        },
    )
}

pub(super) fn entity(
    campaign: i64,
    entity: i64,
    row: &v1_4::EntityPageRow,
    groups: &[v1_4::PartiesRow],
    token: &str,
) -> Html<String> {
    page(
        &row.name,
        html! {
            nav { a href=(format!("/gm/campaigns/{campaign}")) { "Campaign" } }
            p { (row.kind) }
            form method="post" {
                (csrf(token))
                label { "Summary" input name="summary" value=(row.summary) maxlength="1000"; }
                label { "Shared text" textarea name="body" maxlength="64000" { (row.body) } }
                label { "Secret text (GM only)" textarea name="secret" maxlength="64000" { (row.secret) } }
                button { "Save text" }
            }
            section { h2 { "Reveal" }
                form method="post" action=(format!("/gm/campaigns/{campaign}/reveal/{entity}")) {
                    (csrf(token)) (parties(groups)) button { "Reveal to party" }
                }
                p { "Known by: " @for known in &row.known_by { (known.name) " " } }
            }
        },
    )
}

pub(super) fn invite(campaign: i64, link: &str) -> Html<String> {
    page(
        "Player seat created",
        html! {
            p { "Send this link to the player using this laptop's LAN address. Anyone with the link can join this seat." }
            a class="invite" href=(link) { (link) }
            p { "The player sees only discoveries shared with their party." }
            a href=(format!("/gm/campaigns/{campaign}")) { "Back to campaign" }
        },
    )
}

pub(super) fn player(name: &str, rows: &[v1_4::KnownEntitiesRow]) -> Html<String> {
    page(
        "Party discoveries",
        html! {
            p { "Playing as " (name) }
            p { "Refresh this page to see new discoveries." }
            @if rows.is_empty() { p { "Your party has no discoveries yet." } }
            ul { @for row in rows {
                @let (id, name, summary) = match row {
                    v1_4::KnownEntitiesRow::Person { id, name, summary, .. }
                    | v1_4::KnownEntitiesRow::Place { id, name, summary }
                    | v1_4::KnownEntitiesRow::Org { id, name, summary }
                    | v1_4::KnownEntitiesRow::Thing { id, name, summary } => (id, name, summary),
                };
                li { a href=(format!("/play/entities/{}", id.0)) { (name) } " · " (summary) }
            } }
        },
    )
}

pub(super) fn player_entity(row: &v1_4::KnownEntityRow) -> Html<String> {
    page(
        &row.name,
        html! {
            nav { a href="/play" { "Party discoveries" } }
            p { (row.summary) }
            div class="prose" { (row.body) }
        },
    )
}
