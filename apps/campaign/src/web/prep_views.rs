use maud::{Markup, html};

use super::views::csrf;
use crate::v1_6;

pub(super) fn forms(id: i64, entities: &[v1_6::PrepEntitiesRow], token: &str) -> Markup {
    html! {
        section { h2 { "Create entity" }
            p { "Create an entity, then edit its shared and secret text. Invite a player to create their character." }
            @for (kind, title) in [("npc", "NPC"), ("region", "Region"), ("settlement", "Settlement"),
                ("site", "Site"), ("faction", "Faction"), ("party", "Party"), ("item", "Item")] {
                details { summary { "Create " (title) }
                    form method="post" action=(format!("/gm/campaigns/{id}/entities")) {
                        (csrf(token))
                        input type="hidden" name="kind" value=(kind);
                        label { "Name" input name="name" required maxlength="200"; }
                        @if kind != "party" {
                            label { "Summary" input name="summary" maxlength="1000"; }
                        }
                        @if matches!(kind, "npc" | "region" | "settlement" | "site" | "item") {
                            label { @if kind == "item" { "Holder" } @else if kind == "npc" { "Location" } @else { "Parent place" }
                                select name="reference" {
                                    option value="" { "None" }
                                    @for entity in entities.iter().filter(|e| if kind == "item" { e.character } else { e.place }) {
                                        option value=(entity.id.0) { (entity.name) }
                                    }
                                }
                            }
                        }
                        @if kind == "faction" {
                            label { "Agenda (GM only)" textarea name="agenda" maxlength="64000" {} }
                        }
                        button { "Create " (title) " and edit text" }
                    }
                }
            }
        }
    }
}
