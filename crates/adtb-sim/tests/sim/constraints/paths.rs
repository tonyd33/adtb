use super::{agree, refusals};

/// Returns `steps` with each `CHARACTER(n)` replaced by `Some` of the key of character `n` and
/// each `PLACE(n)` by `Some` of the key of place `n`, both narrowed by a match.
fn narrowed(steps: &[(&str, bool)]) -> Vec<(String, bool)> {
    let expand = |s: &str| {
        let mut out = s.to_string();
        for (tag, arms) in [
            (
                "CHARACTER(",
                "| Npc x -> Some { value: key x } | Pc x -> Some { value: key x } | Place _ -> None",
            ),
            (
                "PLACE(",
                "| Npc _ -> None | Pc _ -> None | Place x -> Some { value: key x }",
            ),
        ] {
            while let Some(at) = out.find(tag) {
                let open = at + tag.len();
                let close = open + out[open..].find(')').expect("a closing parenthesis");
                let n = out[open..close].to_string();
                out.replace_range(at..=close, &format!("(match entities.key({n}) {arms})"));
            }
        }
        out
    };
    steps.iter().map(|(s, ok)| (expand(s), *ok)).collect()
}

fn holds(name: &str, decl: &str, steps: &[(&str, bool)]) {
    let steps = narrowed(steps);
    let steps: Vec<(&str, bool)> = steps.iter().map(|(s, ok)| (s.as_str(), *ok)).collect();
    agree(name, &format!("{WORLD}{decl}"), &steps);
}

const WORLD: &str = r"
type Campaign = { id: Key campaigns, name: Text }
relation campaigns : Campaign key id generate sequence
type Entity { id: Key entities, campaign: Key campaigns, name: Text }
  = Npc
  | Pc { player: Text }
  | Place { parent: Option (Key entities.Place) }
relation entities : Entity key id generate sequence
set Characters of entities = Npc | Pc
type Quest { id: Key quests, campaign: Key campaigns, giver: Option (Key entities.Characters) }
  = Open
  | Done { outcome: Text }
relation quests : Quest key id generate sequence
type Discovery { id: Key discoveries, party: Key entities }
  = KnowsEntity { entity: Key entities }
  | KnowsQuest { quest: Key quests }
relation discoveries : Discovery key id generate sequence
type Session = { id: Key sessions, campaign: Key campaigns, number: I64 }
relation sessions : Session key id generate sequence
type Encounter = { id: Key encounters, session: Key sessions }
relation encounters : Encounter key id generate sequence
type Combatant { id: Key combatants, encounter: Key encounters, who: Key entities.Characters }
  = Cast
  | Extra { kind: Text }
relation combatants : Combatant key id generate sequence
";

const GIVER: &str = "constraint giver_in_campaign =
  from q in quests, c in entities.Characters
  where q.giver == Some { value: key c } and c.campaign != q.campaign
  select q
";

const GIVER_FLIPPED: &str = "constraint giver_in_campaign =
  from q in quests, c in entities.Characters
  where Some { value: key c } == q.giver and q.campaign != c.campaign
  select q
";

const GIVER_MATCH: &str = "constraint giver_in_campaign =
  from q in quests
  where match q.giver
    | Some { value } -> value.campaign != q.campaign
    | None -> false
  select q
";

const PARENT: &str = "constraint parent_in_campaign =
  from l in entities.Place, p in entities.Place
  where l.parent == Some { value: key p } and (p.campaign != l.campaign or key p == key l)
  select l
";

const DISCOVERY: &str = "constraint discovery_in_campaign =
  from d in discoveries
  where d.party.campaign != (match d
    | KnowsEntity k -> k.entity.campaign
    | KnowsQuest q -> q.quest.campaign)
  select d
";

const CAST: &str = "constraint cast_in_campaign =
  from c in combatants.Cast
  where c.who.campaign != c.encounter.session.campaign
  select c
";

fn admitted(name: &str, decl: &str) {
    let why = refusals(&format!("{WORLD}{decl}"));
    assert!(why.is_empty(), "{name}: {why:?}");
}

#[test]
fn a_join_through_an_optional_reference_is_admitted() {
    admitted("giver", GIVER);
}

#[test]
fn a_join_through_an_optional_reference_written_the_other_way_round_is_admitted() {
    admitted("giver flipped", GIVER_FLIPPED);
}

#[test]
fn a_match_on_an_optional_reference_is_admitted() {
    admitted("giver match", GIVER_MATCH);
}

#[test]
fn a_join_through_an_optional_self_reference_is_admitted() {
    admitted("parent", PARENT);
}

#[test]
fn paths_inside_the_arms_of_a_match_over_a_sum_are_admitted() {
    admitted("discovery", DISCOVERY);
}

#[test]
fn a_path_through_two_references_is_admitted() {
    admitted("cast", CAST);
}

const SETUP: &str = r#"transact
  insert campaigns { id: campaigns.key(1), name: "one" }
  insert campaigns { id: campaigns.key(2), name: "two" }
  insert entities.Npc { id: entities.key(1), campaign: campaigns.key(1), name: "ann" }
  insert entities.Npc { id: entities.key(2), campaign: campaigns.key(2), name: "bo" }
  insert entities.Pc { id: entities.key(3), campaign: campaigns.key(1), name: "cy", player: "p" }"#;

fn giver_history() -> Vec<(&'static str, bool)> {
    vec![
        (SETUP, true),
        (
            "insert quests.Open { id: quests.key(1), campaign: campaigns.key(1), giver: None }",
            true,
        ),
        (
            "insert quests.Open { id: quests.key(2), campaign: campaigns.key(1), \
             giver: CHARACTER(2) }",
            false,
        ),
        (
            "insert quests.Open { id: quests.key(2), campaign: campaigns.key(1), \
             giver: CHARACTER(1) }",
            true,
        ),
        (
            "update q in quests where key q == quests.key(1) set match q
  | Open o -> Open { o with giver: CHARACTER(2) }
  | Done d -> d",
            false,
        ),
        (
            "update q in quests where key q == quests.key(1) set match q
  | Open o -> Open { o with giver: CHARACTER(3) }
  | Done d -> d",
            true,
        ),
        (
            "update e in entities where key e == entities.key(1) set match e
  | Npc n -> Npc { n with campaign: campaigns.key(2) }
  | Pc p -> p
  | Place l -> l",
            false,
        ),
        (
            "update e in entities where key e == entities.key(3) set match e
  | Npc n -> n
  | Pc p -> Npc { campaign: campaigns.key(2), name: p.name }
  | Place l -> l",
            false,
        ),
        (
            "update e in entities where key e == entities.key(3) set match e
  | Npc n -> n
  | Pc p -> Npc { campaign: p.campaign, name: p.name }
  | Place l -> l",
            true,
        ),
        (
            "update q in quests where key q == quests.key(2) set match q
  | Open o -> Done { campaign: campaigns.key(2), giver: o.giver, outcome: \"x\" }
  | Done d -> d",
            false,
        ),
        (
            "update q in quests where key q == quests.key(2) set match q
  | Open o -> Done { campaign: o.campaign, giver: o.giver, outcome: \"x\" }
  | Done d -> d",
            true,
        ),
        (
            "transact
  delete q in quests where key q == quests.key(2)
  update e in entities where key e == entities.key(1) set match e
    | Npc n -> Npc { n with campaign: campaigns.key(2) }
    | Pc p -> p
    | Place l -> l",
            true,
        ),
    ]
}

#[test]
fn a_join_through_an_optional_reference_holds_on_every_side() {
    holds("giver", GIVER, &giver_history());
}

#[test]
fn a_join_through_an_optional_reference_written_the_other_way_round_holds_on_every_side() {
    holds("giver flipped", GIVER_FLIPPED, &giver_history());
}

#[test]
fn a_match_on_an_optional_reference_holds_on_every_side() {
    holds("giver match", GIVER_MATCH, &giver_history());
}

#[test]
fn a_join_through_an_optional_self_reference_holds_on_every_side() {
    holds(
        "parent",
        PARENT,
        &[
            (SETUP, true),
            (
                r#"insert entities.Place { id: entities.key(10), campaign: campaigns.key(1), name: "realm", parent: None }"#,
                true,
            ),
            (
                r#"insert entities.Place { id: entities.key(11), campaign: campaigns.key(2), name: "town", parent: PLACE(10) }"#,
                false,
            ),
            (
                r#"insert entities.Place { id: entities.key(11), campaign: campaigns.key(1), name: "town", parent: PLACE(10) }"#,
                true,
            ),
            (
                "update e in entities where key e == entities.key(10) set match e
  | Npc n -> n
  | Pc p -> p
  | Place l -> Place { l with parent: PLACE(10) }",
                false,
            ),
            (
                "update e in entities where key e == entities.key(10) set match e
  | Npc n -> n
  | Pc p -> p
  | Place l -> Place { l with campaign: campaigns.key(2) }",
                false,
            ),
            (
                "update e in entities where key e == entities.key(11) set match e
  | Npc n -> n
  | Pc p -> p
  | Place l -> Place { l with parent: None }",
                true,
            ),
            (
                "update e in entities where key e == entities.key(10) set match e
  | Npc n -> n
  | Pc p -> p
  | Place l -> Place { l with campaign: campaigns.key(2) }",
                true,
            ),
        ],
    );
}

#[test]
fn paths_inside_the_arms_of_a_match_over_a_sum_hold_on_every_side() {
    holds(
        "discovery",
        DISCOVERY,
        &[
            (SETUP, true),
            (
                "insert quests.Open { id: quests.key(1), campaign: campaigns.key(1), giver: None }",
                true,
            ),
            (
                "insert discoveries.KnowsEntity { id: discoveries.key(1), party: entities.key(1), \
                 entity: entities.key(2) }",
                false,
            ),
            (
                "insert discoveries.KnowsEntity { id: discoveries.key(1), party: entities.key(1), \
                 entity: entities.key(3) }",
                true,
            ),
            (
                "insert discoveries.KnowsQuest { id: discoveries.key(2), party: entities.key(1), \
                 quest: quests.key(1) }",
                true,
            ),
            (
                "update q in quests where key q == quests.key(1) set match q
  | Open o -> Open { o with campaign: campaigns.key(2) }
  | Done d -> d",
                false,
            ),
            (
                "update e in entities where key e == entities.key(3) set match e
  | Npc n -> n
  | Pc p -> Pc { p with campaign: campaigns.key(2) }
  | Place l -> l",
                false,
            ),
            (
                "update e in entities where key e == entities.key(1) set match e
  | Npc n -> Npc { n with campaign: campaigns.key(2) }
  | Pc p -> p
  | Place l -> l",
                false,
            ),
            (
                "update d in discoveries where key d == discoveries.key(2) set match d
  | KnowsEntity k -> k
  | KnowsQuest q -> KnowsEntity { party: q.party, entity: entities.key(2) }",
                false,
            ),
            (
                "update d in discoveries where key d == discoveries.key(2) set match d
  | KnowsEntity k -> k
  | KnowsQuest q -> KnowsEntity { party: q.party, entity: entities.key(3) }",
                true,
            ),
            (
                "transact
  delete d in discoveries
  update e in entities where key e == entities.key(1) set match e
    | Npc n -> Npc { n with campaign: campaigns.key(2) }
    | Pc p -> p
    | Place l -> l",
                true,
            ),
        ],
    );
}

#[test]
fn a_path_through_two_references_holds_on_every_side() {
    holds(
        "cast",
        CAST,
        &[
            (SETUP, true),
            (
                "transact
  insert sessions { id: sessions.key(1), campaign: campaigns.key(1), number: 1 }
  insert sessions { id: sessions.key(2), campaign: campaigns.key(2), number: 1 }
  insert encounters { id: encounters.key(1), session: sessions.key(1) }
  insert encounters { id: encounters.key(2), session: sessions.key(2) }",
                true,
            ),
            (
                "insert combatants.Cast from (from w in entities.Characters where key w == entities.key(2) \
                 select { id: combatants.key(1), encounter: encounters.key(1), who: key w })",
                false,
            ),
            (
                "insert combatants.Cast from (from w in entities.Characters where key w == entities.key(1) \
                 select { id: combatants.key(1), encounter: encounters.key(1), who: key w })",
                true,
            ),
            (
                "update e in encounters where key e == encounters.key(1) set \
                 Encounter { e with session: sessions.key(2) }",
                false,
            ),
            (
                "update s in sessions where key s == sessions.key(1) set \
                 Session { s with campaign: campaigns.key(2) }",
                false,
            ),
            (
                "update e in entities where key e == entities.key(1) set match e
  | Npc n -> Npc { n with campaign: campaigns.key(2) }
  | Pc p -> p
  | Place l -> l",
                false,
            ),
            (
                r#"update c in combatants where key c == combatants.key(1) set match c
  | Cast k -> Extra { encounter: encounters.key(2), who: k.who, kind: "rat" }
  | Extra x -> x"#,
                true,
            ),
            (
                "update c in combatants where key c == combatants.key(1) set match c
  | Cast k -> k
  | Extra x -> Cast { encounter: encounters.key(2), who: x.who }",
                false,
            ),
            (
                "update c in combatants where key c == combatants.key(1) set match c
  | Cast k -> k
  | Extra x -> Cast { encounter: encounters.key(1), who: x.who }",
                true,
            ),
            (
                "transact
  update s in sessions where key s == sessions.key(2) set \
                 Session { s with campaign: campaigns.key(1) }
  update e in entities where key e == entities.key(2) set match e
    | Npc n -> Npc { n with campaign: campaigns.key(1) }
    | Pc p -> p
    | Place l -> l",
                true,
            ),
        ],
    );
}
