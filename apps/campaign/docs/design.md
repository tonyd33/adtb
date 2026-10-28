# Campaign manager: design

A local-first campaign manager for a tabletop game master. One person,
the GM, writes the campaign: its people, places, factions, quests and
notes, the sessions it is played in, and what happened in them. The
players at the table see a read-only view of the part of that world
their party has discovered. It knows nothing about any game's rules:
no stat blocks, no dice, no character sheets.

It runs as a web server on the GM's laptop, opened in a browser. Phones
and tablets on the same network join as players. One process owns one
database file, which holds every campaign the GM runs.

The schema is `schema/campaign.adt` (release 1.0, as a schema file) and
`schema/migrations/` (the operator log). The registered queries and
transactions are `bundles/campaign-1.5.adb`, pinned at version 5;
`bundles/campaign-1.0.adb` is release 1.0's, pinned at version 1. What
adtb does not yet do for it is in `gaps.md`.

## 1. What it is for

Rivals split into two camps. World Anvil, Kanka and LegendKeeper are
hosted wikis: World Anvil is reported slow at the table, Kanka has no
offline mode, and LegendKeeper is said to sync after working offline. Fantasia Archive and Obsidian vaults are
local and free, but they are document stores: they do not know that a
quest is open or closed, that an NPC died in session 12, or what the
party has been told. This app is local and fast, and it models the
campaign: lifecycles, discovery and time are data the engine checks,
not conventions in prose.

Version 1 covers three jobs.

- Prep: write NPCs, locations, factions and quests, link them from
  notes, and plan the next session.
- At the table: run the session live. Track initiative and encounters,
  find anything in two keystrokes, reveal things to the party as they
  learn them.
- History: the session log, the timeline, what the party knows, and how
  each quest ended.

Maps come in release 1.1 (§10).

## 2. Screens and flows

The app is a set of server-rendered pages. The GM's pages are at `/gm`
on the laptop; a player's are at `/play` on any device that has joined.
Every page has a search box bound to `/` that searches the current
campaign by name.

### 2.1 The GM in prep

- **Campaigns.** A list of campaigns, running or paused, then concluded
  ones with their end date. New campaign asks for a title, a pitch and
  the first party's name.
- **Campaign home.** Four columns: people (NPCs and player characters),
  places, factions and parties, quests by state. Each shows name and
  summary. A button per kind creates one.
- **Entity page.** The entity's name, kind and one-line summary, then
  two text panes: *Shared*, which the party sees once they discover the
  entity, and *Secret*, which only the GM ever sees. Below: what this
  text links to, what links here, the groups it belongs to, and which
  parties know it. Editing is a plain textarea with a live preview
  beside it (§5).
- **Session prep.** The next planned session: its number, title and
  prep text, the encounters prepared for it with their rosters, the
  open quests, and the entities its prep links to. *Start session*
  makes it the live session.
- **Notes.** Free notes for the GM: house rules, ideas, loose threads.
  Notes can link entities; players never see notes.

### 2.2 The GM at the table

The live view is one page, laid out for a laptop screen beside a
rulebook:

- Left: the encounter tracker. Initiative order, one row per slot: the
  label, the standing (ready, downed, fled, slain), and a free-text
  tracker field the GM uses for hit points or conditions in whatever
  form the game uses. The acting slot is highlighted. *Next* advances
  the turn; past the last slot it starts the next round. Rows can be
  added mid-fight.
- Centre: the search result or the entity page last opened, read-only,
  secrets shown.
- Right: a *Reveal* button on every entity, quest and timeline event
  shown, which tells the party; and a quick note box that appends to
  the session log.

Starting an encounter asks for each combatant's roll; the app sorts
them (ties broken by the order they were added) and numbers the slots
1, 2, 3 and so on. A combatant who joins mid-fight takes a free slot
number between two others or after the last; slots may have gaps.
Killing an NPC from the tracker marks it slain in the encounter and
records its death in this session.

*End session* asks for the date, the GM's log and the recap players
will read. It resolves any running encounter.

### 2.3 The GM after the session

- **Quest log.** Rumoured, active, completed and failed quests. Taking
  a rumour, and completing or failing an active quest, records the
  session it happened in and, for the last two, the outcome.
- **Timeline.** Events in campaign order. An event is either backstory,
  set before play, or something that happened in a session. Each event
  has an era label (`Year 412, spring`) the GM writes freely, and a sort
  number that orders the timeline. In-world calendars differ too much
  between games to model in version 1.
- **Sessions.** Every session with its log, recap and the encounters
  that ran.

### 2.4 A player

A player joins by scanning a QR code the GM shows, which opens
`/join/CODE` on the laptop's address. The GM picks which seat the code
is for: a player name, their party and their character's name. Joining
sets a cookie holding the seat's token; leaving the campaign retags
the seat, which revokes it.

The player's pages are read-only and phone-first:

- **Party.** The people, places and groups the party knows, by kind. A
  dead NPC shows as dead only if the party was in the session where it
  died.
- **Entity.** The shared text, with links to entities the party knows
  rendered as links and links to anything else rendered as plain text.
  Nothing from the secret pane, and no "linked from" entry that comes
  from secret text.
- **Quests.** The quests the party has been told about, in their
  current state.
- **Timeline.** The events the party has learned of, in order.
- **Recaps.** Each session the party played, newest first.
- **Now.** During a live session, the initiative order of the running
  encounter: labels and standings, never the GM's tracker text. This
  page updates itself as the GM advances turns.

Every player page updates itself when the GM changes something it
shows (§8.3).

## 3. Data model

One module, `campaign`. Every relation carries its campaign, directly
or through what it references, and constraints keep each fact inside
one campaign.

### 3.1 Relations

| Relation | Element type | Constructors | Notes |
| --- | --- | --- | --- |
| `campaigns` | `Campaign` | `Running`, `Paused { since }`, `Concluded { ended, epilogue }` | |
| `entities` | `Entity` | `Npc`, `Fallen`, `PlayerCharacter`, `Location`, `Faction`, `Party` | header: campaign, name, summary, body, secret |
| `players` | `Player` | `Seated { token }`, `Left { left }` | a human with a seat at the table |
| `memberships` | `Membership` | `Current`, `Former { left_in }` | a character in a faction or party |
| `sessions` | `GameSession` | `Planned`, `Live { started }`, `Played { played_on, log, recap }` | header: party, number, title, prep |
| `quests` | `Quest` | `Rumoured`, `Active`, `Completed`, `Failed` | |
| `events` | `Event` | `Backstory`, `InPlay { session }` | the timeline |
| `involvements` | `Involvement` | one | an entity involved in an event |
| `notes` | `Note` | one | GM-only notes |
| `links` | `Link` | `FromEntity`, `FromSession`, `FromQuest`, `FromEvent`, `FromNote` | one per `[[link]]` in a text |
| `discoveries` | `Discovery` | `KnowsEntity`, `KnowsQuest`, `KnowsEvent` | what a party knows |
| `encounters` | `Encounter` | `Prepared`, `Underway { round, turn }`, `Resolved { rounds, aftermath }` | |
| `combatants` | `Combatant` | `Cast { who }`, `Extra { kind }` | header: label, standing, tracker |
| `initiative` | `Turn` | one | a combatant's slot in a running encounter |

Named constructor sets: `entities.Characters = Npc | PlayerCharacter |
Fallen`, `entities.Groups = Faction | Party`, `sessions.Run = Live |
Played`, `quests.Open = Rumoured | Active`, `quests.Closed = Completed
| Failed`.

### 3.2 Where the sums do the work

- **Kinds are constructors.** An NPC, a place and a faction are
  constructors of one `Entity` sum with one key, so a link, a
  discovery or an involvement points at any entity with one `Key
  entities` field, and search, backlinks and the player view range over
  one relation. Kind-specific fields (an NPC's location, a place's
  parent and scale, a faction's agenda) live on their constructor.
- **Lifecycles are constructors, and a change of state is a retag.**
  A quest is `Rumoured`, `Active`, `Completed` or `Failed`; taking it
  retags it to `Active` with the session it was taken in; closing it
  retags to `Completed` or `Failed` with the session and the outcome.
  A session is `Planned`, `Live`, `Played`. An encounter is
  `Prepared`, `Underway`, `Resolved`. A seat is `Seated` or `Left`. A
  dead NPC is `Fallen`, which keeps the NPC's key, so every link,
  membership and past encounter still reaches it.
- **References are typed by constructor sets.** A location's parent is
  `Option (Key entities.Location)`: it cannot be an NPC. A membership's
  member is `Key entities.Characters` and its group `Key
  entities.Groups`: an NPC joins a faction or a party, never a
  location. A death, a quest's progress and an in-play event reference
  `Key sessions.Run`, a session that is live or played, never one still
  planned. A discovery belongs to `Key entities.Party`. Because
  `Characters` holds `Fallen`, killing an NPC never invalidates a
  reference to it.
- **Uniques per constructor.** `unique (campaign) in Live` allows one
  live session per campaign; `unique (session) in Underway` one running
  encounter per session; `unique (token) in Seated` makes a revoked
  seat's token free.

### 3.3 Constraints

Each is a query that must return nothing; the engine checks every one
at the end of every statement.

| Constraint | Rule |
| --- | --- |
| `player_in_campaign`, `character_player_in_campaign`, `membership_in_campaign`, `session_party_in_campaign`, `death_in_campaign`, `location_parent_in_campaign`, `quest_giver_in_campaign`, `npc_location_in_campaign`, `fallen_location_in_campaign`, `item_holder_in_campaign` | references stay inside one campaign |
| `entity_discovery_in_campaign`, `quest_discovery_in_campaign`, `event_discovery_in_campaign` | a party only discovers things of its own campaign |
| `entity_link_in_campaign` and the four other link constraints | a text only links entities of its own campaign |
| `underway_needs_live_session` | an encounter runs only during a live session |
| `turn_in_its_encounter` | an initiative slot belongs to a combatant of the same encounter |
| `turn_is_seated` | the acting slot of a running encounter exists |
| `cast_in_campaign` | a combatant from the cast is a character of the encounter's campaign |
| `the_dead_stay_dead` | an NPC that died in session n is in no encounter of a later session |
| `closed_quests_stay_closed` | a transition constraint: no quest moves from `Closed` back to `Open` |
| `played_sessions_stay_played` | a transition constraint: a played session is never planned or live again |
| checks `named`, `rounds_count`, `slots_count` | a name is not empty; rounds and slots count from 1 |

A location's parent chain must not loop. That needs recursion, which no
constraint may use, so the app checks it before it writes a parent;
the schema refuses only a location that is its own parent.

## 4. Discovery and the player view

What a party knows is a set of rows in `discoveries`: one per entity,
quest or event, keyed in effect by `(party, thing)` through the
per-constructor uniques. Each records the session it was learned in,
or none when the GM reveals it during prep.

The GM reveals with one click (`reveal`, `reveal_quest`); revealing
twice is a no-op, by an upsert that skips on conflict. Hiding again is
a delete, which the app offers but does not expect to be common.

Every player query takes the party, from the player's seat, and reads
only through `discoveries`:

- `known_entities` joins the party's `KnowsEntity` rows to `entities`
  and returns `Person`, `Place` or `Org`, never the secret text.
- `known_entity` returns one entity's shared text only if the party
  knows it, with its links filtered to entities the party also knows
  and to links found in shared text (`hidden` is false). The page
  renders `[[Name]]` as a link when the target is in that list and as
  plain text otherwise, so the existence of an undiscovered entity is
  never sent.
- `party_quests`, `party_timeline` and `recaps` likewise.

The secret pane, the GM's session log, notes, the encounter tracker's
text and every undiscovered row never leave the server for a player.
This is enforced by the queries the player handlers may call, not by
the database's grants: the GM and the players are not database
principals (§9.4).

## 5. Notes and links

Text is plain, markdown-ish, edited in a textarea. The app renders a
small subset: paragraphs, `#` headings, `*emphasis*`, `**strong**`,
lists, and links.

A link is `[[Name]]` or `[[Name|shown text]]`, where `Name` is an
entity's name in the campaign; `unique (campaign, name)` makes the
name resolve to one entity. While typing `[[`, the editor completes
names from `lookup`. Unknown names render red in the preview and are
offered as *create NPC*, *create place* and so on.

Saving a text is one transaction: it writes the text and replaces the
text's rows in `links`, one per resolved name, marked `hidden` when the
link is in a secret pane (`write_entity`, `write_prep`, `write_note`).
The app parses the links, not the database: there are no triggers, and
the text is the source of truth for what it says.

The `links` rows are what make backlinks and the player filter cheap:
"what links here" is `e.<links.target`, and the player view filters by
it. Renaming an entity (`rename`) keeps its key, so every `links` row
still reaches it; the app then rewrites `[[Old]]` in each text that
links it, found through the same backlinks, one `write_entity` per
text. Until a text is rewritten its `[[Old]]` renders as plain text.

A rich-text editor would add a large front-end dependency for little
gain at the table; the textarea keeps the app small and the stored
text readable outside it.

## 6. Live session state

All live state is in the database, so a crash or a closed laptop lid
loses nothing and the players' views stay consistent with the GM's:

- the live session: the campaign's one `sessions.Live` row;
- the running encounter: its one `encounters.Underway` row, holding
  the round and the acting slot (`turn`);
- the initiative order: the encounter's `initiative` rows, one per
  combatant, with its roll and slot;
- each combatant's standing and tracker text: header fields of
  `combatants`.

`start_encounter` inserts the slots and retags the encounter to
`Underway { round: 1, turn: 1 }` in one statement, so a started
encounter always has its first slot (`turn_is_seated`). `next_turn`
moves `turn` to the least slot greater than it; past the last slot it
moves to the least slot and increments `round`. `join_fight` adds a
slot to a running encounter. `leave_fight` deletes a combatant's slot
and, when it was the acting slot, moves the turn on in the same
statement, which `turn_is_seated` requires. `end_encounter` retags to
`Resolved` with the number of rounds and deletes the slots.

## 7. History and timeline

History is the played sessions, the timeline and the quest outcomes.

- A session's log is the GM's; its recap is the players'. Both are
  written when the session ends, and can be edited later.
- Timeline events are `Backstory` or `InPlay { session }`. Their order
  is the GM's sort number, not the session order, since play often
  reveals the past. `index (campaign, sort)` serves the timeline page.
  Involvements tie events to entities, so an entity page can show its
  history.
- A quest's outcome is the constructor it ended in and the text the GM
  wrote, with the session it closed in. Because closed quests stay
  closed, the log is append-only in effect.
- "What the party knows" is `discoveries`, each with the session it was
  learned in, so the player view can show "learned in session 7".

Reading the past (`at instant`) is not needed in version 1: the log
and the timeline are the history the GM wants.

## 8. Web stack

### 8.1 Server

- **axum on tokio.** axum gives routing, form and query extraction, and
  server-sent events out of the box, on hyper. The engine's calls block,
  so each handler runs its database call on tokio's blocking pool
  (`spawn_blocking`), sized to the engine's workers.
- **maud for HTML.** Templates are Rust macros, checked at compile time,
  with no template files to ship and escaping by default.
- **One binary.** Static assets (one stylesheet, one script) are
  embedded with `include_bytes!`. The binary takes the database path
  and a port, prints the LAN address and the GM's one-time login link.

Fit with `deny.toml`, checked against crates.io on 2026-10-26 (see
`work/kb/axum.md`): axum 0.8 with default features off and tokio 1
resolve to about fifty crates, all MIT, Apache-2.0 or dual, except
`matchit` (axum's router), which is `MIT AND BSD-3-Clause`. So
`deny.toml` needs `BSD-3-Clause` in `[licenses].allow` and an entry per
crate in `[bans].allow`. `ryu` (`Apache-2.0 OR BSL-1.0`) and `memchr`
(`Unlicense OR MIT`) pass on their permitted alternative. The tree
brings `syn` 3 through `serde_derive` and `tokio-macros` and `syn` 2
through `maud_macros`; the workspace's lock already holds both, so
the stack adds no new duplicate. No TLS
crate is needed: the app serves plain HTTP on the LAN.

### 8.2 Front end

Server-rendered HTML with one hand-written script of a few hundred
lines and no framework:

- forms post normally and the server redirects, so every page works
  without the script;
- the script adds the search palette, `[[` completion, the live
  preview, and in-place updates (§8.3);
- the tracker's *Next* button posts with `fetch` and swaps the tracker
  fragment, so the GM never waits for a page load at the table.

### 8.3 Live updates

Server-sent events, not websockets: the updates only flow from server
to browser, SSE reconnects by itself, and axum serves it without
another crate.

Each page subscribes to `/events?campaign=K`. After every committed
transaction the handler publishes, on an in-process broadcast channel
per campaign, the names of the fragments it touched (`tracker`,
`party`, `quests`, `timeline`, `recaps`). A page that shows one of them
fetches that fragment again and swaps it in. A player's fragment
request runs the player query for their party, so the filtering of §4
applies to every update.

Engine feeds (a bundle's change streams) are not needed: every write
comes from this process, which knows what it wrote. When the server
mode lets other processes write, a feed of the touched relations
replaces the in-process channel.

## 9. Talking to adtb

### 9.1 Opening

The binary ships the migrations and the bundle compiled in
(`include_str!`), and opens the file with them, as an embedded
application does: open applies any migrations the file lacks and
registers the bundle. A file from an older release is upgraded on
first open; the release's own older bundle is superseded by that open,
so it holds back none of the release's `enable` operators.

`build.rs` runs `adtb-codegen` over the log and the bundle files. The
library holds what it generates: module `schema` with the current host types,
and modules `v1_0`, `v1_3`, `v1_4` and `v1_5` with each bundle's
`BUNDLE`, error type and one value per query and transaction. The app
opens with `v1_5::BUNDLE.registration()`, binds it with
`Database::bundle`, and calls `bundle.call(v1_5::lookup, (c, q), &opts)`,
getting typed rows back; `tests/engine.rs` does exactly this.

### 9.2 Registered queries and transactions

The browser app calls `bundles/campaign-1.5.adb`, pinned at schema version 5.
The original 1.0, 1.3 and 1.4 bundles remain immutable. Opening registers 1.5
and supersedes earlier app bundles. Migration 0005 adds the missing same-campaign
checks for NPC/Fallen locations and item holders.

| Screen | Queries | Transactions |
| --- | --- | --- |
| Campaigns | `campaigns` | `new_campaign` |
| Campaign home and search | `prep_entities`, `lookup`, `parties`, `quest_log` | `add_npc`, `add_region`, `add_settlement`, `add_site`, `add_faction`, `add_party`, `add_item`, `give_item` |
| Entity page | `entity_page` | `write_entity_text`, `write_entity`, `rename`, `join`, `leave`, `reveal`, `forget` |
| Session prep | `session_board`, `roster` | `plan_session`, `write_prep`, `add_encounter`, `add_cast`, `add_extra` |
| Live view | `live_session`, `tracker`, `roster` | `start_session`, `start_encounter`, `next_turn`, `join_fight`, `leave_fight`, `set_standing`, `kill`, `end_encounter`, `end_session`, `reveal`, `reveal_quest` |
| Quests | `quest_log` | `add_quest`, `take_quest`, `shelve_quest`, `resume_quest`, `close_quest`, `abandon_quest`, `reveal_quest` |
| Maps | `map_view` | `add_map`, `pin_entity`, `add_marker`, `reveal_map` |
| Timeline | `timeline` | `log_backstory`, `log_in_play` |
| Notes | | `write_note` |
| Seats | `seat` | `seat_player`, `unseat` |
| Player pages | `known_entities`, `known_entity`, `party_quests`, `party_timeline`, `recaps`, `party_encounter`, `known_maps` | none |

The prep editor passes parsed shared and secret name sets to `write_entity`.
It resolves names in the source campaign and replaces text and both link sets
inside one registered transaction, including engine retries. Unknown names
produce no link row. `prep_entities` and `parties` list all campaign entities
and parties independently of the limited `lookup` query. The original 1.4
text-only editor's `write_entity_text` leaves link rows untouched.

Every GM action is one registered transaction, so each is one
statement: one change set, checked against every constraint at its
end, retried by the engine on a serialization failure. The app never
holds an interactive transaction open across a request.

### 9.3 Ad hoc text

Ad hoc text is for what the app does not do every day:

- export of a campaign to a document, and import of one;
- the GM's *console*, a hidden page for one-off fixes ("retag every NPC
  of this faction to fallen"), off by default;
- maintenance: `verify`, `backup`.

### 9.4 Principals

The app opens the file as its owner and is the only principal. The GM
and the players are users of the app, not of the database: the GM
signs in with a one-time link printed at start, players with their
seat token. The handlers decide which queries each may call; players'
handlers call only the player queries of §9.2.

### 9.5 Concurrency

The GM may have the laptop and a tablet open; several players refresh
at once. Reads run as read-only transactions and never wait. Writes
are short registered transactions. Where two GM writes collide (two
tabs planning a session, so both compute the same number) one fails
with a serialization failure and the engine reruns it, and its rerun
takes the next number. The app keeps each interactive transaction, when
it uses one, on its own thread: a write that waits on a key another
open transaction inserted waits for that transaction to end.

## 10. Schema evolution

Each release is one or more typed migrations appended to the log. The
1.0 bundle pins version 1; each later release ships its own bundle
pinned to its version.

### 10.1 Release 1.1: maps and items (`0002`)

- `add type Map`, `add relation maps`: an image, its size, its
  campaign.
- `add type Pin = OfEntity { entity } | Marker { label }` with a
  position, `add relation pins`: a pin marks an entity or carries a
  label. Constraints keep pins on their map's image and in its
  campaign.
- `add constructor Discovery.KnowsMap { map }`, a unique per party and
  map, a campaign constraint, then `enable discoveries.KnowsMap`:
  parties discover maps as they discover entities.
- `add constructor Entity.Item { holder: Option (Key
  entities.Characters) }` with its `arm` of `kind_label`, then `enable
  entities.Item`. Items become entities, so they link, are discovered
  and are searched with no change to those queries.

The 1.0 bundle reads `entities` whole, so the readers-first rule holds
`enable entities.Item` until it is released. Embedded, the 1.1 open
supersedes it first, so the migration applies. On a server the
`enable` would go in a migration of its own, applied once the 1.0
bundle is retired.

### 10.2 Release 1.2: kinds of place (`0003`)

`split constructor Entity.Location into Region | Settlement | Site`,
routed by the `scale` each location already has. The routing carries
`parent` whole and fixes `scale` to a nullary constructor in each arm,
so the split is invertible and has a derived merge as its inverse.
References typed `Key entities.Location` (an NPC's location) are
retyped to the three products; `add set Places of entities = Region |
Settlement | Site` names them for later references. `kind_label` gets
an arm per product.

After 1.2, a field can require a settlement, which 1.3's shops would
use. The release opens in the engine, fresh and over 1.1 rows.

### 10.3 Release 1.3: shelved and abandoned quests (`0004`)

- `add constructor Quest.Shelved { taken_in, reason } after Active joins
  quests.Open`, then `enable`: a quest the party set aside, still open.
- `add constructor Quest.Abandoned { closed_in, reason } joins
  quests.Closed`, then `enable`.

Because both join the named sets, `closed_quests_stay_closed` covers
them with no change: an abandoned quest cannot be shelved again. The
1.3 bundle's quest queries add the two arms; the 1.0 to 1.2 bundles,
pinned earlier, never see the new constructors.

### 10.4 Prep reference checks (`0005`)

Add same-campaign constraints for `Npc.location`, `Fallen.location` and
`Item.holder`. Existing valid version-4 files upgrade on open. A file with an
existing violation is refused with its constraint named; migration does not
silently clear or redirect references.

## 11. Load workload

A workload for the simulation harness, run against one file through
the in-process API with the engine's deterministic runtime, at fixed
seeds.

### 11.1 Population

- 200 campaigns, each with 1 to 3 parties of 3 to 6 players, 300 to
  3,000 entities (Zipf-distributed, so a few large campaigns dominate),
  30 to 200 played sessions, 50 to 500 quests, 200 to 2,000 events.
  Entity texts of 200 bytes to 20 kB, each with 0 to 40 links.
- Discoveries: each party knows 20% to 70% of its campaign's entities.

### 11.2 Actors

- **Prepping GMs**, one per campaign not at the table: bursts of
  `write_entity` (half of them renaming or relinking), `add_npc`,
  `plan_session`, `write_prep`, with think time between.
- **Live tables**, 10 campaigns at once: a GM runs a session script.
  Start the session; three to five encounters of 4 to 12 combatants,
  each 3 to 8 rounds of `next_turn`, `set_standing` on most turns, a
  `kill` in a third of encounters; 10 to 30 `reveal` calls; quests
  taken and closed; `end_session`.
- **Players**, 3 to 6 per live table, each refreshing `party_encounter`
  after every turn and `known_entities` or `known_entity` after every
  reveal, as SSE would make them; plus a background rate of
  `party_timeline` and `recaps`.
- **A second GM device** on half the live tables, issuing the same
  kind of writes as the GM at the same moment.

### 11.3 Collisions the workload must cause

- Two devices of one GM run `next_turn` on the same encounter: one must
  fail to serialize and rerun, and the turn must advance exactly twice.
- Two devices run `plan_session` for one campaign at once: both
  sessions commit with distinct numbers after reruns, never a duplicate
  number.
- `kill` of an NPC in one session while the other device adds it to an
  encounter prepared for a later session: whichever commits second is
  refused by `the_dead_stay_dead`.
- `reveal` of the same entity by two devices: one discovery row.
- `start_session` on two planned sessions of one campaign: one live
  session.
- `write_entity` of one entity from two devices: one rerun, last
  writer's text and links agree.

### 11.4 Checks

After the run, and at random points during it under the harness's
fault injection:

- every constraint's query returns nothing (the engine guarantees it;
  the workload confirms it from outside with ad hoc queries);
- every player query's result is a subset of what the GM's queries say
  the party knows; no secret text appears in any player result;
- each encounter's turn sequence, replayed from the workload's log of
  committed `next_turn` calls, matches its `round` and `turn`;
- read latency of `party_encounter` and `known_entity` at the 99th
  percentile stays under 20 ms while the live tables write, the number
  the table can feel.

## 12. Open questions

- Whether the GM wants more than one live session per campaign, for
  split parties. `unique (campaign) in Live` would become `unique
  (party) in Live`.
- Whether player characters belong to players across campaigns. Version
  1 keeps a player and their character inside one campaign.
