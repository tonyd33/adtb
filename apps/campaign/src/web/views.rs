use axum::response::Html;
use maud::{DOCTYPE, Markup, html};

use crate::v1_6;

pub(super) fn page(title: &str, content: Markup) -> Html<String> {
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

pub(super) fn csrf(token: &str) -> Markup {
    html! { input type="hidden" name="csrf" value=(token); }
}

fn parties(rows: &[v1_6::PartiesRow]) -> Markup {
    html! { label { "Party" select name="party" required {
        @for party in rows { option value=(party.id.0) { (party.name) } }
    } } }
}

pub(super) fn home(rows: &[v1_6::CampaignsRow], token: &str) -> Html<String> {
    page(
        "Campaigns",
        html! {
            ul { @for row in rows {
                @let (id, title) = match row {
                    v1_6::CampaignsRow::Ongoing { id, title } | v1_6::CampaignsRow::Over { id, title, .. } => (id, title),
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
    groups: &[v1_6::PartiesRow],
    entities: &[v1_6::PrepEntitiesRow],
    query: &str,
    token: &str,
) -> Html<String> {
    page(
        title,
        html! {
            nav { a href="/gm" { "All campaigns" } " · " a href=(format!("/gm/campaigns/{id}/sessions")) { "Sessions" } }
            form method="get" { label { "Find an entity" input name="q" value=(query); } button { "Search" } }
            ul { @for row in entities.iter().filter(|row| row.matches_query) { li {
                a href=(format!("/gm/campaigns/{id}/entities/{}", row.id.0)) { (row.name) }
                " · " (row.kind) " · " (row.summary)
            } } }
            section { h2 { "Invite player" }
                form method="post" action=(format!("/gm/campaigns/{id}/seats")) {
                    (csrf(token)) (parties(groups))
                    label { "Player name" input name="name" required maxlength="200"; }
                    label { "Character name" input name="character" required maxlength="200"; }
                    button { "Create seat link" }
                }
            }
            (super::prep_views::forms(id, entities, token))
        },
    )
}

pub(super) fn entity(
    campaign: i64,
    entity: i64,
    row: &v1_6::EntityPageRow,
    groups: &[v1_6::PartiesRow],
    token: &str,
) -> Html<String> {
    page(
        &row.name,
        html! {
            nav { a href=(format!("/gm/campaigns/{campaign}")) { "Campaign" } }
            p { (row.kind) }
            @if let Some(reference) = &row.reference {
                p { "Location, parent or holder: " a href=(format!("/gm/campaigns/{campaign}/entities/{}", reference.id.0)) { (reference.name) } }
            }
            @if !row.agenda.is_empty() { section { h2 { "Agenda (GM only)" } div class="prose" { (row.agenda) } } }

            form method="post" {
                (csrf(token))
                label { "Summary" input name="summary" value=(row.summary) maxlength="1000"; }
                label { "Shared text" textarea name="body" maxlength="64000" { (row.body) } }
                label { "Secret text (GM only)" textarea name="secret" maxlength="64000" { (row.secret) } }
                button { "Save text" }
            }
            p { "Use [[Name]] or [[Name|label]] to link entities in this campaign. Unresolved names are marked below." }
            section { h2 { "Shared preview" }
                div class="prose" { (super::links::render(&row.body,
                    &row.mentions.iter().filter(|m| !m.hidden).map(|m| (m.target.0, m.name.as_str())).collect::<Vec<_>>(),
                    &format!("/gm/campaigns/{campaign}/entities"), true)) }
                h2 { "Secret preview (GM only)" }
                div class="prose" { (super::links::render(&row.secret,
                    &row.mentions.iter().filter(|m| m.hidden).map(|m| (m.target.0, m.name.as_str())).collect::<Vec<_>>(),
                    &format!("/gm/campaigns/{campaign}/entities"), true)) }
            }
            section { h2 { "Mentions" }
                ul { @for mention in &row.mentions { li {
                    a href=(format!("/gm/campaigns/{campaign}/entities/{}", mention.target.0)) { (mention.name) }
                    @if mention.hidden { " (secret)" } @else { " (shared)" }
                } } }
                h2 { "Linked from" }
                ul { @for source in &row.mentioned_in { li {
                    @match source {
                        v1_6::EntityPageRowMentionedIn::InEntity { id, name } => { a href=(format!("/gm/campaigns/{campaign}/entities/{}", id.0)) { (name) } }
                        v1_6::EntityPageRowMentionedIn::InSession { number, .. } => { "Session " (number) }
                        v1_6::EntityPageRowMentionedIn::InQuest { title, .. }
                        | v1_6::EntityPageRowMentionedIn::InEvent { title, .. }
                        | v1_6::EntityPageRowMentionedIn::InNote { title, .. } => { (title) }
                    }
                } } }
                h2 { "Groups" }
                ul { @for group in &row.groups { li { (group.name) " · " (group.role) @if !group.current { " (former)" } } } }
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

pub(super) fn player(name: &str, rows: &[v1_6::KnownEntitiesRow]) -> Html<String> {
    page(
        "Party discoveries",
        html! {
            nav { a href="/play/now" { "Now" } " · " a href="/play/recaps" { "Recaps" } }
            p { "Playing as " (name) }
            p { "Refresh this page to see new discoveries." }
            @if rows.is_empty() { p { "Your party has no discoveries yet." } }
            ul { @for row in rows {
                @let (id, name, summary) = match row {
                    v1_6::KnownEntitiesRow::Person { id, name, summary, .. }
                    | v1_6::KnownEntitiesRow::Place { id, name, summary }
                    | v1_6::KnownEntitiesRow::Org { id, name, summary }
                    | v1_6::KnownEntitiesRow::Thing { id, name, summary } => (id, name, summary),
                };
                li { a href=(format!("/play/entities/{}", id.0)) { (name) } " · " (summary) }
            } }
        },
    )
}

pub(super) fn player_entity(row: &v1_6::KnownEntityRow) -> Html<String> {
    page(
        &row.name,
        html! {
            nav { a href="/play" { "Party discoveries" } }
            p { (row.summary) }
            div class="prose" { (super::links::render(&row.body,
                &row.mentions.iter().map(|m| (m.target.0, m.name.as_str())).collect::<Vec<_>>(),
                "/play/entities", false)) }
            section { h2 { "Linked from" }
                ul { @for source in &row.mentioned_by { li {
                    a href=(format!("/play/entities/{}", source.source.0)) { (source.name) }
                } } }
            }
        },
    )
}
