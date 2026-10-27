bundle campaign-1.3 pins 4 (55331b63f9c889135c04e7d2570d67e27d89050ac449f462cf659e85af6f79d3) edition "2026-10"
import campaign

query campaigns () -> Set (Ongoing { id: Key campaigns, title: Text } | Over { id: Key campaigns, title: Text, ended: Date }) =
  from c in campaigns
  select match c
    | Running r | Paused r -> Ongoing { id: r.id, title: r.title }
    | Concluded d -> Over { id: d.id, title: d.title, ended: d.ended }

query lookup (c: Key campaigns, q: Text) -> Set ({ id: Key entities, name: Text, kind: Text, summary: Text }) =
  from e in top 20 by (e.name, key e) (
    from e in entities
    where e.campaign == c and contains(fold(e.name), fold(q))
    select e)
  select { id: e.id, name: e.name, kind: kind_label(e), summary: e.summary }
  order by e.name

query entity_page (k: Key entities) -> Set ({ name: Text, kind: Text, summary: Text, body: Text, secret: Text,
    mentions: Set ({ target: Key entities, name: Text, hidden: Bool }),
    mentioned_in: Set (InEntity { id: Key entities, name: Text }
                     | InSession { id: Key sessions, number: I64 }
                     | InQuest { id: Key quests, title: Text }
                     | InEvent { id: Key events, title: Text }
                     | InNote { id: Key notes, title: Text }),
    groups: Set ({ group: Key entities.Groups, name: Text, role: Text, current: Bool }),
    known_by: Set ({ party: Key entities.Party, name: Text }) }) =
  from e in entities
  where key e == k
  select {
    name: e.name,
    kind: kind_label(e),
    summary: e.summary,
    body: e.body,
    secret: e.secret,
    mentions: (from l in links.FromEntity where l.entity == k
               select { target: l.target, name: l.target.name, hidden: l.hidden }),
    mentioned_in: (from l in e.<links.target
                   select match l
                     | FromEntity x -> InEntity { id: x.entity, name: x.entity.name }
                     | FromSession x -> InSession { id: x.session, number: x.session.number }
                     | FromQuest x -> InQuest { id: x.quest, title: x.quest.title }
                     | FromEvent x -> InEvent { id: x.event, title: x.event.title }
                     | FromNote x -> InNote { id: x.note, title: x.note.title }),
    groups: (from m in memberships where m.member == k
             select { group: m.group, name: m.group.name, role: m.role,
                      current: match m | Current _ -> true | Former _ -> false }),
    known_by: (from d in discoveries.KnowsEntity where d.entity == k
               select { party: d.party, name: d.party.name })
  }

query session_board (s: Key sessions) -> Set ({ number: I64, title: Text, prep: Text,
    encounters: Set ({ id: Key encounters, name: Text, state: Tag Encounter }),
    open_quests: Set ({ id: Key quests, title: Text, active: Bool }),
    cast: Set ({ id: Key entities, name: Text }) }) =
  from x in sessions
  where key x == s
  select {
    number: x.number,
    title: x.title,
    prep: x.prep,
    encounters: (from e in encounters where e.session == s
                 select { id: e.id, name: e.name, state: tag(e) }),
    open_quests: (from q in quests.Open where q.campaign == x.campaign
                  select { id: q.id, title: q.title,
                           active: match q | Active _ -> true | Rumoured _ -> false | Shelved _ -> false }),
    cast: (from l in links.FromSession where l.session == s
           select { id: l.target, name: l.target.name })
  }

query live_session (c: Key campaigns) -> Set ({ id: Key sessions, number: I64, title: Text,
    fighting: Set ({ id: Key encounters, name: Text }) }) =
  from s in sessions.Live
  where s.campaign == c
  select {
    id: s.id,
    number: s.number,
    title: s.title,
    fighting: (from e in encounters.Underway where e.session == key s
               select { id: e.id, name: e.name })
  }

query roster (k: Key encounters) -> Set ({ id: Key combatants, label: Text, standing: Standing, tracker: Text,
    who: Option (Key entities.Characters) }) =
  from c in combatants
  where c.encounter == k
  select { id: c.id, label: c.label, standing: c.standing, tracker: c.tracker,
           who: match c | Cast x -> Some { value: x.who } | Extra _ -> None }

query tracker (k: Key encounters) -> Set ({ name: Text, round: I64, turn: I64,
    order: Set ({ slot: I64, roll: I64, combatant: Key combatants, label: Text, standing: Standing, tracker: Text }) }) =
  from e in encounters.Underway
  where key e == k
  select {
    name: e.name,
    round: e.round,
    turn: e.turn,
    order: (from t in initiative where t.encounter == k
            select { slot: t.slot, roll: t.roll, combatant: t.combatant, label: t.combatant.label,
                     standing: t.combatant.standing, tracker: t.combatant.tracker })
  }

query quest_log (c: Key campaigns) -> Set (Rumour { id: Key quests, title: Text, hook: Text }
                                         | Taken { id: Key quests, title: Text, since: I64 }
                                         | Aside { id: Key quests, title: Text, since: I64, reason: Text }
                                         | Done { id: Key quests, title: Text, won: Bool, outcome: Text, closed: I64 }
                                         | Dropped { id: Key quests, title: Text, reason: Text, closed: I64 }) =
  from q in quests
  where q.campaign == c
  select match q
    | Rumoured r -> Rumour { id: r.id, title: r.title, hook: r.hook }
    | Active a -> Taken { id: a.id, title: a.title, since: a.taken_in.number }
    | Completed d -> Done { id: d.id, title: d.title, won: true, outcome: d.outcome, closed: d.closed_in.number }
    | Failed f -> Done { id: f.id, title: f.title, won: false, outcome: f.outcome, closed: f.closed_in.number }
    | Shelved h -> Aside { id: h.id, title: h.title, since: h.taken_in.number, reason: h.reason }
    | Abandoned x -> Dropped { id: x.id, title: x.title, reason: x.reason, closed: x.closed_in.number }

query timeline (c: Key campaigns) -> Set Event =
  from v in events
  where v.campaign == c
  select v
  order by (v.sort, key v)

query seat (token: Text) -> Set ({ player: Key players, name: Text, party: Key entities.Party, campaign: Key campaigns }) =
  from p in players.Seated
  where p.token == token
  select { player: key p, name: p.name, party: p.party, campaign: p.campaign }

query known_entities (p: Key entities.Party) -> Set (Person { id: Key entities, name: Text, summary: Text, dead: Bool }
                                                   | Place { id: Key entities, name: Text, summary: Text }
                                                   | Org { id: Key entities, name: Text, summary: Text }
                                                   | Thing { id: Key entities, name: Text, summary: Text }) =
  from d in discoveries.KnowsEntity, e in entities
  where d.party == p and d.entity == key e
  select match e
    | Npc n | PlayerCharacter n -> Person { id: n.id, name: n.name, summary: n.summary, dead: false }
    | Fallen f -> Person { id: f.id, name: f.name, summary: f.summary, dead: f.died_in.party == p }
    | Region l | Settlement l | Site l -> Place { id: l.id, name: l.name, summary: l.summary }
    | Faction g | Party g -> Org { id: g.id, name: g.name, summary: g.summary }
    | Item i -> Thing { id: i.id, name: i.name, summary: i.summary }

query known_entity (p: Key entities.Party, k: Key entities) -> Set ({ name: Text, summary: Text, body: Text,
    mentions: Set ({ target: Key entities, name: Text }),
    mentioned_by: Set ({ source: Key entities, name: Text }) }) =
  from e in entities
  where key e == k and knows(p, k)
  select {
    name: e.name,
    summary: e.summary,
    body: e.body,
    mentions: (from l in links.FromEntity
               where l.entity == k and not l.hidden and knows(p, l.target)
               select { target: l.target, name: l.target.name }),
    mentioned_by: (from l in links.FromEntity
                   where l.target == k and not l.hidden and knows(p, l.entity)
                   select { source: l.entity, name: l.entity.name })
  }

query party_quests (p: Key entities.Party) -> Set (Rumour { id: Key quests, title: Text, hook: Text }
                                                 | Taken { id: Key quests, title: Text, since: I64 }
                                                 | Aside { id: Key quests, title: Text, since: I64, reason: Text }
                                                 | Done { id: Key quests, title: Text, won: Bool, outcome: Text, closed: I64 }
                                                 | Dropped { id: Key quests, title: Text, reason: Text, closed: I64 }) =
  from d in discoveries.KnowsQuest, q in quests
  where d.party == p and d.quest == key q
  select match q
    | Rumoured r -> Rumour { id: r.id, title: r.title, hook: r.hook }
    | Active a -> Taken { id: a.id, title: a.title, since: a.taken_in.number }
    | Completed x -> Done { id: x.id, title: x.title, won: true, outcome: x.outcome, closed: x.closed_in.number }
    | Failed x -> Done { id: x.id, title: x.title, won: false, outcome: x.outcome, closed: x.closed_in.number }
    | Shelved x -> Aside { id: x.id, title: x.title, since: x.taken_in.number, reason: x.reason }
    | Abandoned x -> Dropped { id: x.id, title: x.title, reason: x.reason, closed: x.closed_in.number }

query party_timeline (p: Key entities.Party) -> Set ({ id: Key events, title: Text, body: Text, era: Text, sort: I64 }) =
  from d in discoveries.KnowsEvent, v in events
  where d.party == p and d.event == key v
  select { id: v.id, title: v.title, body: v.body, era: v.era, sort: v.sort }
  order by v.sort

query recaps (p: Key entities.Party) -> Set ({ number: I64, title: Text, played_on: Date, recap: Text }) =
  from s in sessions.Played
  where s.party == p
  select { number: s.number, title: s.title, played_on: s.played_on, recap: s.recap }
  order by s.number desc

query party_encounter (p: Key entities.Party) -> Set ({ name: Text, round: I64, turn: I64,
    order: Set ({ slot: I64, label: Text, standing: Standing }) }) =
  from s in sessions.Live, e in encounters.Underway
  where s.party == p and e.session == key s
  select {
    name: e.name,
    round: e.round,
    turn: e.turn,
    order: (from t in initiative where t.encounter == key e
            select { slot: t.slot, label: t.combatant.label, standing: t.combatant.standing })
  }

query map_view (m: Key maps) -> Set ({ title: Text, image: Text, width: I64, height: I64,
    pins: Set (OnEntity { id: Key pins, x: I64, y: I64, entity: Key entities, name: Text }
             | Label { id: Key pins, x: I64, y: I64, label: Text }) }) =
  from x in maps
  where key x == m
  select {
    title: x.title,
    image: x.image,
    width: x.width,
    height: x.height,
    pins: (from p in pins where p.map == m
           select match p
             | OfEntity o -> OnEntity { id: o.id, x: o.x, y: o.y, entity: o.entity, name: o.entity.name }
             | Marker k -> Label { id: k.id, x: k.x, y: k.y, label: k.label })
  }

query known_maps (p: Key entities.Party) -> Set ({ id: Key maps, title: Text, image: Text }) =
  from d in discoveries.KnowsMap, x in maps
  where d.party == p and d.map == key x
  select { id: x.id, title: x.title, image: x.image }

txn new_campaign (title: Text, pitch: Text, party: Text) -> Key campaigns =
  transact
    let c = insert campaigns.Running { title: title, pitch: pitch }
    insert entities.Party { campaign: key c, name: party }
    return key c

txn add_npc (c: Key campaigns, name: Text, summary: Text, at: Option (Key entities.Places)) -> Key entities =
  transact
    let n = insert entities.Npc { campaign: c, name: name, summary: summary, location: at }
    return key n

txn add_region (c: Key campaigns, name: Text, summary: Text, parent: Option (Key entities.Places)) -> Key entities =
  transact
    let l = insert entities.Region { campaign: c, name: name, summary: summary, parent: parent }
    return key l

txn add_settlement (c: Key campaigns, name: Text, summary: Text, parent: Option (Key entities.Places)) -> Key entities =
  transact
    let l = insert entities.Settlement { campaign: c, name: name, summary: summary, parent: parent }
    return key l

txn add_site (c: Key campaigns, name: Text, summary: Text, parent: Option (Key entities.Places)) -> Key entities =
  transact
    let l = insert entities.Site { campaign: c, name: name, summary: summary, parent: parent }
    return key l

txn add_faction (c: Key campaigns, name: Text, summary: Text, agenda: Text) -> Key entities =
  transact
    let f = insert entities.Faction { campaign: c, name: name, summary: summary, agenda: agenda }
    return key f

txn add_party (c: Key campaigns, name: Text) -> Key entities =
  transact
    let p = insert entities.Party { campaign: c, name: name }
    return key p

txn add_item (c: Key campaigns, name: Text, summary: Text, holder: Option (Key entities.Characters)) -> Key entities =
  transact
    let i = insert entities.Item { campaign: c, name: name, summary: summary, holder: holder }
    return key i

txn give_item (i: Key entities.Item, holder: Option (Key entities.Characters)) -> () =
  transact
    update x in entities.Item where key x == i set Item { x with holder: holder }

txn add_map (c: Key campaigns, title: Text, image: Text, width: I64, height: I64) -> Key maps =
  transact
    let m = insert maps { campaign: c, title: title, image: image, width: width, height: height }
    return key m

txn pin_entity (m: Key maps, x: I64, y: I64, e: Key entities) -> Key pins =
  transact
    let p = insert pins.OfEntity { map: m, x: x, y: y, entity: e }
    return key p

txn add_marker (m: Key maps, x: I64, y: I64, label: Text) -> Key pins =
  transact
    let p = insert pins.Marker { map: m, x: x, y: y, label: label }
    return key p

txn reveal_map (p: Key entities.Party, m: Key maps, s: Option (Key sessions.Run)) -> () =
  transact
    insert discoveries.KnowsMap { party: p, learned_in: s, map: m }
    unless conflict on (party, map) in KnowsMap
    else skip

txn seat_player (c: Key campaigns, name: Text, party: Key entities.Party, token: Text, character: Text) -> Key players =
  transact
    let p = insert players.Seated { campaign: c, name: name, party: party, token: token }
    let pc = insert entities.PlayerCharacter { campaign: c, name: character, player: key p }
    insert memberships.Current { member: key pc, group: party, role: "member" }
    return key p

txn unseat (p: Key players, day: Date) -> () =
  transact
    update x in players.Seated where key x == p set Left { left: day }

txn join (m: Key entities.Characters, g: Key entities.Groups, role: Text) -> () =
  transact
    insert memberships.Current { member: m, group: g, role: role }

txn leave (m: Key entities.Characters, g: Key entities.Groups, s: Option (Key sessions.Run)) -> () =
  transact
    update x in memberships.Current where x.member == m and x.group == g
    set Former { left_in: s }

txn write_entity (e: Key entities, summary: Text, body: Text, secret: Text, shown: Set (Key entities), hidden: Set (Key entities)) -> () =
  transact
    update x in entities where key x == e
    set match x
      | Npc y -> Npc { y with summary: summary, body: body, secret: secret }
      | Fallen y -> Fallen { y with summary: summary, body: body, secret: secret }
      | PlayerCharacter y -> PlayerCharacter { y with summary: summary, body: body, secret: secret }
      | Region y -> Region { y with summary: summary, body: body, secret: secret }
      | Settlement y -> Settlement { y with summary: summary, body: body, secret: secret }
      | Site y -> Site { y with summary: summary, body: body, secret: secret }
      | Faction y -> Faction { y with summary: summary, body: body, secret: secret }
      | Item y -> Item { y with summary: summary, body: body, secret: secret }
      | Party y -> Party { y with summary: summary, body: body, secret: secret }
    delete l in links.FromEntity where l.entity == e
    insert links.FromEntity from (from t in shown select { target: t, hidden: false, entity: e })
    insert links.FromEntity from (from t in hidden select { target: t, hidden: true, entity: e })

txn rename (e: Key entities, name: Text) -> () =
  transact
    update x in entities where key x == e
    set match x
      | Npc y -> Npc { y with name: name }
      | Fallen y -> Fallen { y with name: name }
      | PlayerCharacter y -> PlayerCharacter { y with name: name }
      | Region y -> Region { y with name: name }
      | Settlement y -> Settlement { y with name: name }
      | Site y -> Site { y with name: name }
      | Faction y -> Faction { y with name: name }
      | Item y -> Item { y with name: name }
      | Party y -> Party { y with name: name }

txn reveal (p: Key entities.Party, e: Key entities, s: Option (Key sessions.Run)) -> () =
  transact
    insert discoveries.KnowsEntity { party: p, learned_in: s, entity: e }
    unless conflict on (party, entity) in KnowsEntity
    else skip

txn forget (p: Key entities.Party, e: Key entities) -> () =
  transact
    delete d in discoveries.KnowsEntity where d.party == p and d.entity == e

txn reveal_quest (p: Key entities.Party, q: Key quests, s: Option (Key sessions.Run)) -> () =
  transact
    insert discoveries.KnowsQuest { party: p, learned_in: s, quest: q }
    unless conflict on (party, quest) in KnowsQuest
    else skip

txn plan_session (c: Key campaigns, party: Key entities.Party, title: Text) -> Key sessions =
  transact
    let s = insert sessions.Planned {
      campaign: c,
      party: party,
      number: 1 + count(x for x in sessions where x.campaign == c),
      title: title,
      planned_for: None
    }
    return key s

txn write_prep (s: Key sessions, prep: Text, shown: Set (Key entities)) -> () =
  transact
    update x in sessions where key x == s
    set match x
      | Planned y -> Planned { y with prep: prep }
      | Live y -> Live { y with prep: prep }
      | Played y -> Played { y with prep: prep }
    delete l in links.FromSession where l.session == s
    insert links.FromSession from (from t in shown select { target: t, hidden: true, session: s })

txn start_session (s: Key sessions) -> () =
  transact
    update x in sessions.Planned where key x == s set Live { }

txn end_session (s: Key sessions, played_on: Date, log: Text, recap: Text) -> () =
  transact
    update x in sessions.Live where key x == s
    set Played { played_on: played_on, log: log, recap: recap }
    update e in encounters.Underway where e.session == s
    set Resolved { rounds: e.round, aftermath: "" }
    delete t in initiative where t.encounter.session == s

txn add_encounter (s: Key sessions, name: Text, setup: Text) -> Key encounters =
  transact
    let e = insert encounters.Prepared { session: s, name: name, setup: setup }
    return key e

txn add_cast (k: Key encounters, who: Key entities.Characters, label: Text) -> Key combatants =
  transact
    let c = insert combatants.Cast { encounter: k, label: label, who: who }
    return key c

txn add_extra (k: Key encounters, label: Text, kind: Text) -> Key combatants =
  transact
    let c = insert combatants.Extra { encounter: k, label: label, kind: kind }
    return key c

txn start_encounter (k: Key encounters, rolls: Set ({ combatant: Key combatants, roll: I64, slot: I64 })) -> () =
  transact
    insert initiative from (from o in rolls
                            select { encounter: k, combatant: o.combatant, roll: o.roll, slot: o.slot })
    update e in encounters.Prepared where key e == k
    set Underway { round: 1, turn: 1 }

txn next_turn (k: Key encounters) -> () =
  transact
    update e in encounters.Underway where key e == k
    set match min(t.slot for t in initiative where t.encounter == k and t.slot > e.turn)
      | Some { value } -> Underway { e with turn: value }
      | None -> match min(t.slot for t in initiative where t.encounter == k)
        | Some { value } -> Underway { e with round: e.round + 1, turn: value }
        | None -> e

txn join_fight (k: Key encounters, c: Key combatants, roll: I64, slot: I64) -> () =
  transact
    insert initiative { encounter: k, combatant: c, roll: roll, slot: slot }

txn leave_fight (c: Key combatants) -> () =
  transact
    delete t in initiative where t.combatant == c
    update e in encounters.Underway
    where exists t in initiative where t.combatant == c and t.encounter == key e and t.slot == e.turn
    set match min(u.slot for u in initiative where u.encounter == key e and u.combatant != c and u.slot > e.turn)
      | Some { value } -> Underway { e with turn: value }
      | None -> match min(u.slot for u in initiative where u.encounter == key e and u.combatant != c)
        | Some { value } -> Underway { e with round: e.round + 1, turn: value }
        | None -> e

txn set_standing (c: Key combatants, standing: Standing, tracker: Text) -> () =
  transact
    update x in combatants where key x == c
    set match x
      | Cast y -> Cast { y with standing: standing, tracker: tracker }
      | Extra y -> Extra { y with standing: standing, tracker: tracker }

txn kill (n: Key entities.Npc, s: Key sessions.Run) -> () =
  transact
    update x in entities.Npc where key x == n
    set Fallen { died_in: s, location: x.location }
    update c in combatants.Cast where c.who == n
    set Cast { c with standing: Slain }

txn end_encounter (k: Key encounters, aftermath: Text) -> () =
  transact
    update e in encounters.Underway where key e == k
    set Resolved { rounds: e.round, aftermath: aftermath }
    delete t in initiative where t.encounter == k

txn add_quest (c: Key campaigns, title: Text, hook: Text, giver: Option (Key entities.Characters), s: Option (Key sessions.Run)) -> Key quests =
  transact
    let q = insert quests.Rumoured { campaign: c, title: title, hook: hook, giver: giver, heard_in: s }
    return key q

txn take_quest (q: Key quests, s: Key sessions.Run) -> () =
  transact
    update x in quests.Rumoured where key x == q set Active { taken_in: s }

txn close_quest (q: Key quests, s: Key sessions.Run, won: Bool, outcome: Text) -> () =
  transact
    update x in quests.Open where key x == q
    set match won
      | True -> Completed { closed_in: s, outcome: outcome }
      | False -> Failed { closed_in: s, outcome: outcome }

txn shelve_quest (q: Key quests, reason: Text) -> () =
  transact
    update x in quests.Active where key x == q set Shelved { taken_in: x.taken_in, reason: reason }

txn resume_quest (q: Key quests) -> () =
  transact
    update x in quests.Shelved where key x == q set Active { taken_in: x.taken_in }

txn abandon_quest (q: Key quests, s: Key sessions.Run, reason: Text) -> () =
  transact
    update x in quests.Open where key x == q set Abandoned { closed_in: s, reason: reason }

txn log_backstory (c: Key campaigns, title: Text, body: Text, era: Text, sort: I64, involved: Set (Key entities)) -> Key events =
  transact
    let v = insert events.Backstory { campaign: c, title: title, body: body, era: era, sort: sort }
    insert involvements from (from x in involved select { event: key v, entity: x })
    return key v

txn log_in_play (s: Key sessions.Run, title: Text, body: Text, era: Text, sort: I64, involved: Set (Key entities)) -> Key events =
  transact
    let v = insert events.InPlay { campaign: s.campaign, title: title, body: body, era: era, sort: sort, session: s }
    insert involvements from (from x in involved select { event: key v, entity: x })
    return key v

txn write_note (c: Key campaigns, title: Text, body: Text, shown: Set (Key entities)) -> Key notes =
  transact
    let n = insert notes { campaign: c, title: title, body: body }
    insert links.FromNote from (from t in shown select { target: t, hidden: true, note: key n })
    return key n
