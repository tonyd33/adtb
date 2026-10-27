# The first application on adtb

Date: 2026-10-26. Market facts below were gathered by web search and a few
page reads on this date and are recorded in `work/kb/` (files named after
each product: `partsbox.md`, `bomist.md`, `legendkeeper.md`,
`eu-e-invoicing.md` and others). Facts marked [V] there were read on the
vendor's page; [S] came from search snippets or review aggregators and may
be stale. Demand estimates in this report are my inference, not measured.

## Recommendation in one paragraph

Build **Partcrate**, a local-first desktop app for electronics parts
inventory and small-batch builds: parts, where they sit, what arrived, what
each board needs, and what a build will consume. It is aimed at serious
hobbyists and one-to-three person hardware sellers who build the same boards
repeatedly. Sell it as a one-time $59 license with a year of updates. The
domain is a natural fit for adtb: parts come in several shapes, every stock
lot and every build moves through a lifecycle, and the rules that matter
("you cannot consume stock reserved for another build", "a build marked
built has consumed its whole BOM") are exactly the denial constraints and
constructor-typed references the engine enforces. The incumbents are either
hosted subscriptions priced for companies (PartsBox paid tiers start at
EUR 49/month) or a desktop subscription (BOMIST MAKER $15/month billed
yearly), and the free options are self-hosted web servers (InvenTree,
Part-DB) or unmaintained (PartKeepr). The runner-up is a local-first
tabletop RPG campaign manager.

## How the candidates were judged

Three requirements, in the order given:

1. **Sum types do real work.** Entities with several shapes, lifecycles as
   constructors, references that must point at a particular constructor
   set, engine-enforced constraints, and a schema that will change across
   the app's own releases.
2. **Passive income for a solo developer.** Low support burden, no regulated
   domain, sells once or for a small subscription, fits an embedded
   database, and has real evidence of willingness to pay plus a gap in what
   exists.
3. **Buildable by coding agents in Rust on a pre-alpha engine**, with a
   workload worth stress testing under simulated load.

Each is scored 1 to 5. The total weights them 3, 2, 1 to reflect the order.

| # | Candidate | R1 sums | R2 market | R3 build and load | Weighted |
| --- | --- | --- | --- | --- | --- |
| 1 | Electronics parts inventory and builds | 5 | 4 | 4 | 27 |
| 2 | Tabletop RPG campaign manager | 4 | 4 | 3 | 23 |
| 3 | Equipment checkout for small organisations | 5 | 3 | 4 | 25 |
| 4 | Freelancer time tracking and billing | 5 | 2 | 4 | 23 |
| 5 | Fiction story bible | 3 | 3 | 3 | 18 |
| 6 | Handmade seller inventory and COGS | 4 | 2 | 3 | 19 |
| 7 | Homebrew batch log | 4 | 2 | 4 | 20 |
| 8 | Home maintenance and asset log | 3 | 2 | 4 | 17 |
| 9 | Garden planner and seed inventory | 3 | 3 | 2 | 17 |
| 10 | Collection tracker (comics, records, books) | 2 | 2 | 3 | 13 |
| 11 | 3D printing filament and print farm | 3 | 1 | 2 | 13 |

Equipment checkout scores close to the winner on paper, but it is a B2B
product that needs several concurrent front-desk users, which pulls in the
server and a sales process; I rank the RPG manager second instead because it
is the better passive-income product (see its section).

## The candidates

### 1. Electronics parts inventory and small-batch builds (recommended)

**Sum types.** Parts are a relation-level sum: a resistor has resistance,
tolerance and package; a capacitor has capacitance, voltage rating and
dielectric; a semiconductor has a manufacturer part number and pinout; a
generic part has only a category. Stock lots move from on hand to reserved
for a build to consumed, or are scrapped. Builds go from planned to kitting
to built, or are cancelled. Purchase orders go from draft to ordered to
received. A reservation must reference a build that is kitting, which is
`Key builds.Kitting`; when the build retags to `Built`, every reservation
pointing at it is rechecked, so the transaction that finishes a build is
forced to turn its reservations into consumptions in the same change set.
That is the model doing the work a hand-written application would do in
service code and occasionally get wrong. Quantities give aggregate denial
constraints ("reserved quantity on a lot exceeds the lot"). Real schema
changes are easy to foresee: moisture-sensitivity tracking on lots, sub-
assemblies (a part constructor that is itself a build product), new part
kinds, finer storage locations.

**Market.** PartsBox: free Maker plan for one user, then EUR 49, 99, 239 and
499 per month billed annually, hosted only, no offline option [V]. BOMIST:
free up to 100 parts, MAKER $15/month and PRO $50/month billed yearly,
desktop with local workspaces, perpetual and offline licenses only at
Enterprise from $350/month [V]. InvenTree is MIT, free and capable, but is a
self-hosted Django server [S]; Part-DB is likewise a self-hosted web app
[S]; PartKeepr is unmaintained [S]. On EEVblog, BOMIST users ask for a
perpetual license, hierarchical storage locations (rack, drawer, slot),
sub-assemblies and variants, and distributor barcode scanning; one reports
15-second delays creating a part on a fast machine; another values that it
"runs locally, so no server nor setup required" [V]. On Hacker News people
describe home-made SQLite parts databases with exactly the operations this
app needs: take inventory, scan a shipment, populate a board [S]. The gap is
clear: nothing local, fast, and one-time priced between free self-hosted
servers and company subscriptions.

Demand is a niche, not a mass market. My estimate (inference) is tens of
thousands of people who would consider such a tool, of whom a few thousand
would pay; at $59 that is a modest but real passive income if the product
is good and ranked for "electronics parts inventory". Distribution: own site
with Paddle (5% plus $0.50, merchant of record, so no VAT registration [S])
or Gumroad (about 10% [S]); listings on alternativeto.net, the EEVblog and
Hackaday communities, r/electronics, and the KiCad community. Setapp is
possible later for the macOS build (70% of a member's fee shared by usage
[S]) but the audience is cross-platform and Linux-heavy, so it is not the
primary channel.

**Regulation.** None for the hobby and small-seller tier. Lot control and
audit trails for medical or aerospace customers are where PartsBox charges
EUR 499/month; Partcrate should stay out of that tier.

**Build and load.** Tables, forms, CSV import, barcode input (USB scanners
type into the focused field) and PDF labels. No rich text, no maps, no
network service. The workload is a little TPC-C: receives, reservations,
consumptions, cycle counts and reports, with hot rows (everyone uses 0.1 uF
capacitors), so it exercises SSI conflicts and incremental constraint checks
honestly.

### 2. Tabletop RPG campaign manager (runner-up)

**Sum types.** Campaign entities are a sum (NPC, location, faction, item,
deity, session); quests go rumoured, active, completed, failed; a "located
in" field must point at a location and a "member of" field at a faction or
party, which are constructor-set references. Constraints are fewer and
softer (an NPC who is dead cannot appear in a later session's attendance),
and most of the value is in prose.

**Market.** Large and willing to pay: LegendKeeper charges $9/month or
$90/year with only the owner paying [V]; World Anvil from about $54/year,
with complaints that it is slow at the table, cluttered and spammy in its
marketing [S]; Kanka is free and open source with paid tiers from $4.99/month
and no offline mode [S]. But the free floor is strong: Kanka's free tier,
Fantasia Archive (free, GPLv3, offline desktop with about twenty record types
[S]) and Obsidian with community templates. itch.io and the DMs Guild are
natural channels for a $29 to $39 one-time app.

**Build and load.** Needs a good rich-text editor with inline entity links,
and users will expect maps; both are slow to build well. The workload is
light: one person typing, so the load test would be synthetic rather than
the app's own pattern.

### 3. Equipment checkout for small organisations

Film schools, makerspaces, camera rental shops, libraries of things. Items
are available, checked out, in repair or retired; reservations must not
overlap, which is the engine's exclusion constraint over an interval index;
a checkout must reference a member who holds the right certification. Strong
fit and an excellent load test (double booking under concurrency is the
textbook SSI case). The price gap is wide: Cheqroom is $184 to $367 per admin
per month [S], Booqable $29 to $149/month [S]. Against it: buyers are
organisations, so sales calls, invoices and procurement forms; several
staff need it at once, so it is a server product, not embedded; support load
is higher. A good second app, not a good first passive one.

### 4. Freelancer time tracking and billing

Invoices are the canonical sum (Draft, Issued, Paid, Void), time entries are
unbilled or billed on a particular issued invoice, and a payment must
reference an issued invoice: perfect for requirement 1. Requirement 2 fails.
The market is crowded at near-zero prices (Clockify free for five users,
paid from $3.99 per seat; Harvest free for one user and two projects [S]),
and the one-time desktop slot is taken (Timemator $39 [S]; Tyme
$59.99/year [S]). Issuing invoices drags in tax rules, and EU structured
e-invoicing is arriving for small businesses: Belgium for all VAT-registered
businesses from January 2026, Germany issuing for everyone from January
2028, France for micro businesses by 2027 or 2028 [S]. A billing app that
cannot emit Peppol or Factur-X would be unsellable there within two years.

### 5. Fiction story bible

Characters, places, scenes with states (outlined, drafted, revised), series
continuity. Writers do pay once: Scrivener $59.99 [S], Plottr base $150
lifetime [S], Campfire $375 for all modules lifetime [S], and they voice
subscription fatigue [S]. It overlaps with the RPG manager but needs a
manuscript editor on top, and the sum-type payoff is weaker than in either
inventory or RPGs.

### 6. Handmade seller inventory and COGS

Materials, recipes, batches, products, sales. Good sums. Craftybase charges
$20 to $41/month [S], but rivals already undercut it (Inventora with a free
tier, Assemblified from $6/month, Ardent Seller from $0 [S]). The product
only works if it syncs Etsy and Shopify orders, which means API churn and
support tickets, and COGS reports exist for tax filing, which edges into the
regulated area. Sortly's users complain of 77 to 93% price increases [S],
which shows demand for a one-time alternative, but the integration burden
makes it a poor solo passive product.

### 7. Homebrew batch log

Batches move through planned, brewing, fermenting, conditioning, packaged or
dumped; ingredient inventory is consumed by batches. Good sums and a decent
workload. The market is held by Brewfather at about $29.99/year [S] and
BeerSmith 4 at $19.95/year [S], and the value is mostly brewing calculations
(gravity, IBU, water chemistry) rather than records. I believe the hobby is
flat or shrinking [U].

### 8. Home maintenance and asset log

Assets (appliance, vehicle, system), tasks scheduled, due, done or skipped,
warranties. HomeZada lists $99 and $189/year [S], and a crowd of small new
apps compete (Mendlog, Steadwell, HomeBeacon, DwellPulse [S]). Low
willingness to pay, weak constraints, weak load.

### 9. Garden planner and seed inventory

Seed lots, plantings that are sown, transplanted, harvested or failed, beds
by season. Seedtime $168/year [S], GrowVeg $35/year [S]. The selling point
is a visual bed layout plus a crop and frost-date database, which are the
expensive parts to build and carry no sum-type benefit.

### 10. Collection tracker

CLZ charges $19.99/year per collection category [S]. The product's value is
its barcode and catalog database, which is licensed data, not schema. Sum
types help little; the hard part is data acquisition.

### 11. 3D printing filament and print farm

Spoolman is free and integrates with OctoPrint, Klipper and Moonraker [S];
SimplyPrint is free for two printers and cheap above that [S]. The value is
printer integration, which is an endless support surface.

## The recommendation in detail

### Name

**Partcrate.** A web search for "Partcrate" returned nothing relevant
(2026-10-26). Rejected: "KitBin" is a registered trademark for industrial
storage bins, which is adjacent to inventory; "Benchstock" is an existing
offline inventory app for military maintenance teams; "Reelkeep" and
"Reelwise" are taken by unrelated apps. A trademark search (USPTO, EUIPO)
and a domain check are still needed before committing.

### Target user

The serious electronics hobbyist with a few thousand parts in drawers and
reels, and the one-to-three person hardware business (kit sellers, small-run
boards sold online) that builds the same boards in batches of 10 to 500.
Both outgrow a spreadsheet, refuse to run a Django server, and will not pay
EUR 49 a month.

### Price

$59 one-time, which includes a year of updates; the app keeps working
forever on the last version received. After a year, $29 renews updates
(the Sketch or Timemator style of license). A free tier capped at 200 parts
lets people try it with real data. A later "Pro" at $129 one-time can carry
sub-assemblies and lot tracking once they exist. Sold through Paddle on the
app's own site for macOS, Windows and Linux, with offline license-key
checks so the app never phones home to run.

### First version scope (deliberately small)

1. Parts catalog with four kinds (resistor, capacitor, semiconductor,
   generic), manufacturer part numbers, a parametric search per kind, and
   user attributes as a typed key-value relation for anything the kinds do
   not cover.
2. Storage locations as a tree (room, shelf, drawer, bin) and stock lots
   placed in them.
3. Receiving: import a distributor order CSV (Digi-Key, Mouser, LCSC) or scan
   the 2D barcode on the bag, creating lots.
4. Projects with a BOM imported from a KiCad BOM CSV; a shortage report for
   building N units.
5. Builds: plan, kit (reserve lots), mark built (consume) or cancel
   (release). Undo is a compensating change set, not a history rewrite.
6. Labels as PDF for a bin or a lot, and full CSV export of everything.

Not in version 1: live distributor pricing APIs, multi-user sync, sales
orders, purchase-order sending, moisture sensitivity, sub-assemblies. Each
of these is a later release and, conveniently, a real migration.

### GUI stack

Tauri: a web front end gives mature data grids, forms and PDF preview that
egui, iced and Slint do not yet match for a table-heavy business app, while
adtb stays embedded in the Rust core.

### What it exercises in adtb

- **Relation-level sums.** `parts` partitioned into `Resistor`,
  `Capacitor`, `Semiconductor`, `Generic` with one key across them and a
  shared header (manufacturer, MPN); `lots` into `OnHand`, `Reserved`,
  `Consumed`, `Scrapped`; `builds` into `Planned`, `Kitting`, `Built`,
  `Cancelled`; `orders` into `Draft`, `Ordered`, `Received`.
- **Value-level sums.** `Package = Smd { code } | ThroughHole { pitch } |
  Module`, `Dielectric = C0G | X7R | X5R | Y5V`, attribute values as
  `Number | Text | Flag | Quantity`.
- **References to constructor sets.** `Reserved.build: Key builds.Kitting`,
  `Consumed.build: Key builds.Built`, a received order line referencing
  `Key orders.Ordered`. Retagging a build is the state change, and the
  engine's recheck forces reservations to resolve in the same change set.
- **Constraints as denial queries.** Reserved quantity on a lot never
  exceeds its quantity; a built build has consumed every BOM line times the
  quantity; no lot sits in a location that is not a leaf; MPN plus
  manufacturer unique across the semiconductor constructors. These are
  aggregates and inclusions that must be admitted with index-bounded
  incremental checks, which is a real test of that admission rule.
- **Exhaustive `match`.** Every screen that lists parts or lots matches on
  every constructor, so adding a kind in a later release makes the
  registered queries fail registration until each is handled.
- **Typed migrations as product releases.** 1.1 adds moisture tracking:
  a new lot constructor `Opened { at }` and a `Baked` transition, enabled
  readers-first. 1.2 adds sub-assemblies: a new part constructor
  `Assembly { project }` whose stock is produced by builds. 1.3 splits
  `Generic` into `Connector` and `Electromechanical` with a data-moving
  migration. Each user's file upgrades on launch; the app's bundles pin
  versions (`partcrate-1.1`, `partcrate-1.2`).
- **Embedding.** One process, one file, serializable transactions, crash
  safety; backup is copying a snapshot.

### Stress workload

A simulated small contract assembler, run by the load harness against the
same schema and registered transactions the app uses: 50,000 parts,
500,000 lots, 2,000 projects, 32 worker threads mixing receive, plan, kit,
consume, cancel, cycle count and stock move, plus long readers running the
valuation and shortage reports. A few hundred hot parts concentrate
contention, so SSI aborts and retries happen at realistic rates. The oracle
is a set of invariants checked after the run (stock conservation: received
equals on hand plus reserved plus consumed plus scrapped per part), and the
same workload replays across each migration to check that upgrades preserve
them at scale. Later, the team-server mode (several users through adtb's
server) reuses the workload unchanged.

### Main risks

1. **Engine trust.** An inventory app that loses data loses its users at
   once, and adtb will be young. Mitigate with automatic snapshot backups on
   launch, a continuous CSV export option, and a long internal dogfooding
   period on the load harness before release.
2. **The free floor.** PartsBox's free Maker plan and InvenTree are good
   enough for many hobbyists. Partcrate must win on being local, instant and
   one-time, and on builds; it should target the people who build boards
   repeatedly, not the drawer-of-parts collector who will never pay.
3. **Niche size.** The market is thousands of buyers, not millions. Good for
   low support, bad if Tony needs more than modest income from it.
4. **Format churn.** Distributor CSV layouts and barcode payloads change,
   and KiCad BOM exports vary. Keep importers as small, tested, versioned
   parsers with a manual column-mapping fallback, so a format change is a
   patch release rather than a support queue.
5. **Custom fields pressure.** Users will want their own part kinds. A user
   attribute relation covers most of it; resist turning user-defined kinds
   into runtime schema changes in version 1, though it is a tempting later
   showcase for typed migrations.
6. **Desktop distribution costs.** Code signing and notarisation for macOS
   and Windows, and an updater. These are fixed yearly costs and a few days
   of setup, not ongoing support.
7. **Tauri brings TypeScript.** The UI half is not Rust. Agents handle it,
   but the repo gains a second toolchain.

## Runner-up

A local-first tabletop RPG campaign manager, sold one-time at $29 to $39 on
itch.io and its own site: offline, fast at the table, typed entities with
links, quest lifecycles. It has the larger audience and proven willingness
to pay (LegendKeeper $90/year), but strong free competitors, a rich-text
editor and maps to build before it is credible, and a workload too light to
stress the engine on its own terms. If Partcrate's niche looks too small
after a trademark and demand check, this is the one to build.
