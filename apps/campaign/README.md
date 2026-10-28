# Campaign

A local campaign server that exercises adtb through generated, registered calls.
Prep covers creating a campaign, parties, NPCs, places, factions and items,
linking shared and secret text, inviting players, and revealing discoveries.

From the repository root in the devenv shell:

```sh
cargo run --manifest-path apps/campaign/Cargo.toml -- campaign.adt 0.0.0.0:3000
```

The database path defaults to `campaign.adt`, and the listen address defaults
to `0.0.0.0:3000`. Use `127.0.0.1:3000` to keep it on this laptop only.
The server prints a one-use GM login link. Open it, create a campaign with its
first party, then open **Create NPC** and choose **Create NPC and edit text**.
Creation saves the NPC's name and summary; **Save text** is a separate action that saves the shared and
secret panes. Each action is one registered transaction.

On the campaign page, create a player seat and send the seat link to the
player using the laptop's LAN address, for example
`http://192.168.1.20:3000/join/TOKEN`. The startup player address follows the
system's selected network route; check it when using a VPN or several network
interfaces. A player joins in a separate browser/device and initially sees no
undiscovered NPCs. Reveal the NPC to their party on its GM page, then refresh
the player's page. The player sees shared text; the secret pane stays on the
GM's pages. Anyone possessing a seat link can use that seat.

Stop with Ctrl-C to close the database cleanly. Restart with the same path to
keep the campaigns, text, discoveries and player seat tokens. Existing player
cookies continue to work; the new startup link signs in the GM again. A GM
login link is consumed once, and GM cookies expire on a server restart.
The app registers `campaign-1.6`, pinned at schema version 5. Migration 0005
checks that NPC and fallen-character locations and item holders belong to the
same campaign. Prior migrations and bundles stay immutable; open supersedes
the prior app registration while preserving valid data. An older file with a
cross-campaign reference is refused with the offending constraint named; no
rows are repaired silently.

Cookies are HttpOnly and SameSite=Strict. GM writes also require the form's
CSRF token. The server serves plain HTTP for use on a trusted local network.

Use the **Create entity** forms for regions, settlements, sites, factions,
parties and items. NPCs can have a location, places a parent and items a holder.
Inviting a player creates their player character. Entity pages show related
places/holders, faction agendas, groups, mentions and backlinks.

Write `[[Name]]` or `[[Name|label]]` in shared or secret text to link an entity
of the same campaign. **Save text** resolves all names inside one registered
transaction and replaces text and link rows atomically. Repeated names produce
one link per pane; a target in both panes retains both visibility entries.
Unknown names remain plain text, marked in the GM preview. Players receive
links and backlinks only for discoveries their party knows, from shared text.
An alias of an undiscovered target is plain text. Renaming through an external
call keeps stored keys but old name spellings render as plain text until edited.

Open **Sessions** from the campaign page to plan a session for a party, write
GM prep, and start play. Prepare an encounter, add existing characters or
extras, then enter a roll for every combatant. Higher rolls act first; ties
keep creation order. **Next turn** advances the acting slot and starts a new
round after the last slot. Standings and free tracker text persist in the file.
End an encounter with its aftermath, or end the session with a date, GM log and
player recap. Ending a session also resolves any running encounter.

Players use **Now** for labels, standings and the current turn, and **Recaps**
for their party's played sessions. These pages never receive prep, setup, GM
logs or tracker text. Refresh manually to see updates. Existing seat revocation
applies to each request. Played-session GM pages retain prep, logs and recaps.

This slice uses escaped text, normal forms, previews after saving and manual
player refresh. Rich Markdown, completion, automatic updates, midfight roster
changes, death recording, quest/timeline screens and simulated load remain
future work.

Run the HTTP flow tests with:

```sh
cargo nextest run --manifest-path apps/campaign/Cargo.toml --test web \
  --config-file .config/nextest.toml
```

They use real database files and cover authorization, CSRF, invalid input,
concurrent reads and writes, player-secret filtering, repeat reveals and
persistence across reopen. `check` runs these alongside the engine and
campaign tests, lint and dependency policy.
