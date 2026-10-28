# Campaign

A local campaign server that exercises adtb through generated, registered calls.
The first browser flow covers creating a campaign and party, creating an NPC,
editing shared and secret text, inviting a player, and revealing the NPC.

From the repository root in the devenv shell:

```sh
cargo run --manifest-path apps/campaign/Cargo.toml -- campaign.adt 0.0.0.0:3000
```

The database path defaults to `campaign.adt`, and the listen address defaults
to `0.0.0.0:3000`. Use `127.0.0.1:3000` to keep it on this laptop only.
The server prints a one-use GM login link. Open it, create a campaign with its
first party, then choose **Create NPC and edit text**. Creation saves the NPC's
name and summary; **Save text** is a separate action that saves the shared and
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
The app registers `campaign-1.4`, pinned at the same schema version 4 as
`campaign-1.3`. Prior bundle files stay immutable; open supersedes the prior
app registration while preserving its data. No fifth schema migration is
needed for the browser calls.

Cookies are HttpOnly and SameSite=Strict. GM writes also require the form's
CSRF token. The server serves plain HTTP for use on a trusted local network.

This slice uses plain escaped text, normal forms and manual player refresh.
Link editing/rendering, live updates, session tools, quests and history pages
remain future work. Editing text preserves existing link rows.

Run the HTTP flow tests with:

```sh
cargo nextest run --manifest-path apps/campaign/Cargo.toml --test web \
  --config-file .config/nextest.toml
```

They use real database files and cover authorization, CSRF, invalid input,
concurrent reads and writes, player-secret filtering, repeat reveals and
persistence across reopen. `check` runs these alongside the engine and
campaign tests, lint and dependency policy.
