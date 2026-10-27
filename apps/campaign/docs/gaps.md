# Gaps found by the campaign manager

What writing the campaign manager's schema, migrations and bundle
against adtb found, in the language, the engine and the in-process API.
Each entry gives what was written, what happened, what was expected,
and what the spec says. Entries are ordered by how much they cost the
application. The tests that show each one are in `tests/front_end.rs`
and `tests/engine.rs`; the ones that fail are listed at the end.

Found on 2026-10-26, against the workspace at the commit after
`docs: record the campaign manager's users, platform and first scope`.

## Engine: admission

### Joins through an optional reference are not admitted

Resolved 2026-10-26: the planner now splits joins through a reference on constructor equalities (`admission-path-forms`).

Written, in migration 0001:

```adt fragment
add constraint quest_giver_in_campaign =
  from q in quests, c in entities.Characters
  where q.giver == Some { value: key c } and c.campaign != q.campaign
  select q
```

with `giver: Option (Key entities.Characters)`. The same shape guards a
location's parent (`location_parent_in_campaign`).

What happened: opening the file refuses migration 1 with `constraint
campaign::quest_giver_in_campaign is not admitted: changes to
campaign::quests: no field connects them to
campaign::entities.(Npc | Fallen | PlayerCharacter), so every row of it
is read`. Written first with `match q.giver | Some { value } ->
value.campaign != q.campaign | None -> false`, it was refused too, for
changes to the characters.

Expected: admitted. A change to `quests` reaches the character by its
key; a change to a character reaches its quests through the implied
index on `giver`.

Spec: constraints.md §2.1 gives every reference field an implied index
and says it is the index through which a reference is within the
bound; model.md §8.2 makes `Option (Key r.S)` a reference; §5.2 admits
joins whose correlations are covered by indexes. So the spec says these
are admitted.

Cost: the release 1.0 log does not open. `tests/engine.rs` opens it
with these two constraints left out (`UNADMITTED`); `release_1_0_opens`
opens the real log and fails.

### Paths inside the arms of a `match` over a sum are not admitted

Resolved 2026-10-26: the delta compiler now unfolds dereferences and keeps arm conditions (`admission-path-forms`).

Written first:

```adt fragment
constraint discovery_in_campaign =
  from d in discoveries
  where d.party.campaign != (match d
    | KnowsEntity k -> k.entity.campaign
    | KnowsQuest q -> q.quest.campaign
    | KnowsEvent v -> v.event.campaign)
  select d
```

and the same for `links` over its five source constructors.

What happened: refused, `changes to campaign::entities: no field
connects them to campaign::discoveries, so every row of it is read`,
once for each target relation.

Expected: admitted. Each arm is a path through a reference field of its
constructor, and each has an implied index.

Spec: queries.md §5.6 makes a `match` over a relation a union of arm
queries over constructor relations, and §7.2 makes a path shorthand for
a generator and an equality on the key. Each arm query is admitted on
its own: the schema now states one constraint per constructor
(`entity_discovery_in_campaign`, `quest_discovery_in_campaign`, ...),
which the engine admits. That loses the match-scoped constraint whose
`arm` clause release 1.1 would have extended; 1.1 adds
`map_discovery_in_campaign` instead.

### A path of two references is not admitted, the same joins written out are

Resolved 2026-10-26: path forms through several references now admit the same way as the written-out join (`admission-path-forms`).

Written first: `c.who.campaign != c.encounter.session.campaign` over
`combatants.Cast`, and `c.encounter.session.number > f.died_in.number`
for the dead NPC rule.

What happened: refused, `changes to campaign::sessions: no field
connects them to campaign::combatants.Cast`.

Written instead, admitted:

```adt fragment
constraint cast_in_campaign =
  from c in combatants.Cast, e in encounters, s in sessions
  where c.encounter == key e and e.session == key s and c.who.campaign != s.campaign
  select c
```

Expected: the path form admitted like the join form. Spec: queries.md
§7.2 defines the path as that join.

## Language: schema evolution

### An `arm` clause into a parenthesised match corrupts the declaration

Resolved 2026-10-26: the arm is now placed inside the parentheses, after the last arm (`migration-text-corruption`).

Written, against the first form of `discovery_in_campaign` above:

```adt fragment
add constructor Discovery.KnowsMap { map: Key entities }
  arm discovery_in_campaign: KnowsMap m -> m.map.campaign
```

What happened: the migration is refused with `expected select, found |`.
The folded constraint reads:

```text
  where d.party.campaign != (match d
    | KnowsEntity k -> k.entity.campaign
    | KnowsQuest q -> q.quest.campaign
    | KnowsEvent v -> v.event.campaign)
  | KnowsMap m -> m.map.campaign
  select d
```

The arm is placed after the closing parenthesis.

Expected: the arm inside the parentheses, after the last arm. Spec:
evolution.md §4.2, the `arm` clause adds an arm to the match whose
scrutinee covers the new constructor, wherever it sits.

### Splitting a constructor that references itself

Resolved 2026-10-26: split and merge now type the routing at the retyped types and drop retyping edits that fall inside a replaced constructor's own text (`migration-text-corruption`).

That fixed the front end only. The engine still refused release 1.2 at open: `constraint campaign::location_parent_in_campaign is not admitted: changes to campaign::entities.(Region | Settlement | Site): index (parent) in campaign::entities.(Region | Settlement | Site) is missing`. Each product holds its own `parent` field with its own implied index, and the bounded check looked for one index over all three. Resolved 2026-10-26 (`campaign-release-1-2-refused-by-engine`): a read of a constructor set that no one index covers goes through each constructor's own index, as constraints.md §5.1 allows. The full 1.2 and 1.3 logs open in the engine, fresh and over 1.1 rows.

Written, release 1.2 (`schema/migrations/0003.adt`):

```adt fragment
split constructor Entity.Location
  into Region { parent: Option (Key entities.(Region | Settlement | Site)) }
     | Settlement { parent: Option (Key entities.(Region | Settlement | Site)) }
     | Site { parent: Option (Key entities.(Region | Settlement | Site)) }
  by match
    | Location { parent, scale: Realm } -> Region { parent: parent }
    | Location { parent, scale: Town } -> Settlement { parent: parent }
    | Location { parent, scale: Spot } -> Site { parent: parent }
```

What happened: refused, `expected Option (Key entities.(Region |
Settlement | Site)), found Option (Key entities.Location)` at the first
routing arm. The routing is typed with the old field types, and the
retyping of references to the split constructor is not applied to the
value it carries.

Written with the products' `parent` typed `Option (Key
entities.Location)` instead: the operator applies, but the folded
`Entity` declaration is corrupt and fails to parse (`expected a
declaration, found ettlement`):

```text
  | Region { parent: Option (Key entities.Location) }
  | Settlement { parent: Option (Key entities.Location) }
  | Site { parent: Option (Key entities.Location) }ettlement | Site)) }
```

The retyping of `Npc.location` and `Fallen.location` to
`Key entities.(Region | Settlement | Site)` is right; the edit of the
split constructor's own text overlaps it.

Expected: the first form applies. Every row keeps its key, the carried
`parent` value now references one of the three products, and the
products' field types name them.

Spec: evolution.md §4.6 retypes `Key r.OAuth` to `Key r.(Google |
Apple)` and lets a routing arm carry a field whole. It does not say how
the products declare a reference to the constructor being split, or
that the routing types a carried field at its retyped type. The spec
should say both; the text corruption is a bug whichever is chosen.

Cost, until both fixes: release 1.2 did not apply, and so neither did
the full log of 1.3; the tests checked 0004 on top of 0002 alone. They
now apply the full logs.

### Within one migration, only `Key r` may name a relation added later

Resolved 2026-10-26: `Key r.C` and `Key r.(Set)` may now name a relation added later in the same migration, and a printed schema replays as a first migration (`spec-migration-authoring-gaps`).

Written first, as one migration with types in reading order: `type
Player { ..., party: Key entities.Party }` before `relation entities`,
and `Location { parent: Option (Key entities.Location) }` inside
`Entity` itself.

What happened: refused, `entities is added by a later operator, so only
Key entities may name it`.

Spec: evolution.md §3.2 says exactly this. It is a spec choice, not an
engine fault, but it has two consequences the spec does not mention:

- A relation-level sum cannot declare a constructor-typed reference to
  itself in the operator that adds its type, and a cycle whose
  references are constructor-typed on both sides cannot be written as
  `add type` operators alone. Migration 0001 works around it: `Entity`
  is added without `Npc`, `Fallen` and `Location.parent`, then `add
  field Entity.Location.parent ... fill None`, `add constructor
  Entity.Npc`, and, after `sessions` exists, `add constructor
  Entity.Fallen { died_in: Key sessions.Run, ... }`, each enabled in
  the same migration.
- The printed schema (`schema/campaign.adt`) is not a migration: putting
  `add` before each declaration does not apply. A schema printed by
  the engine cannot be replayed as a first migration for a new file.

### `add field` cannot declare a delete action

Resolved 2026-10-26: `add field` now takes a `field_type`, so `on delete ...` may follow its type, and `alter field` changes the action of an existing reference field (`spec-migration-authoring-gaps`).

`field_op` takes a `type`, and a delete action is part of `field_type`,
not of `type` (syntax.md §4, §8.3). So `Location.parent`, added by `add
field` in the workaround above, is `restrict`, and there is no operator
that gives an existing reference field an action. The spec should allow
`on delete ...` after the type of `add field`, or offer an operator for
it.

## In-process API

### No bundles, calls or feeds

`Database::open` takes a path, a log and options. It takes no bundles,
and there is no `Database::bundle`, `Bundle::call` or
`Bundle::subscribe`. The spec has all three (embedding.md §2.1, §3,
§4.1, §4.4). The engine is pre-alpha and this is known; the tests run
each bundle body as ad hoc text with typed parameters.

Running a registered query as ad hoc text is not the same as calling
it. Ad hoc text has no signature, so arms are not checked against the
declared result: `known_entities` built `Person { id: key n, ... }` in
the arm for `Npc | PlayerCharacter` and `Person { id: key f, ... }` in
the arm for `Fallen`, which joins under its signature but has no
common type ad hoc (`Key entities.(Npc | PlayerCharacter)` against
`Key entities.Fallen` inside an anonymous constructor). That matches
queries.md §4.2 and typing.md §5.2, so it is not a fault; the bundle
now uses the header field `id`, typed `Key entities`, in every result.

### No deadline on a call or transaction

`CallOptions::deadline` and `limit`, and the same on `TxnOptions`, are
refused (`does not run in this engine yet`). A write that waits on a
key another open transaction inserted (transactions.md §6.3) cannot be
bounded. The tests found it by holding two interactive transactions on
one thread: the second insert waits for the first to end and the
thread never gets there. Waiting is right by the spec; the
application gives every interactive transaction its own thread.

## Language: friction the spec intends

Not faults. Each costs the application code the spec could save it.

- Updating header fields of a row of any constructor needs a `match`
  with one arm per constructor, each `C { x with ... }`
  (`write_entity`, `write_prep`): `update ... set e` takes a whole new
  value, and a constructor expression names one constructor
  (writes.md §3.2, syntax.md §7). Six arms to change three text fields,
  and every new constructor adds one to each such transaction.
- A single-row insert into a sum whose constructor depends on a
  parameter has no form: `insert r { ... }` needs one constructor
  (writes.md §2.1), and `let` does not take a write `match`
  (syntax.md §7). A transaction that inserts such a row and then uses
  its key is two transactions (`log_backstory`, `log_in_play`).
- A path stops at an `Option` (queries.md §7.2), so a constraint over
  an optional reference is a join with `Some { value: key c }`, which
  then meets the admission gap above.

## Failing tests

As of 2026-10-26, with `cargo nextest run` in `apps/campaign`: 25 of 25
tests pass. No test fails.

The schema file checks, is the fold of migration 0001 and replays as a
first migration; releases 1.1 to 1.3 apply in the front end; all 50
registered queries and transactions check at version 1; and in the
engine both constraints are admitted, every release opens fresh,
1.1 upgrades 1.0 rows, 1.2 and 1.3 route 1.1 locations by scale and
keep a split location's parent in its campaign, and the app runs prep,
discovery, an encounter, a death, a closed quest, a shelved quest and
two colliding planners as the design describes.

With admission and migration text fixed, the app could now restore the
match-scoped `discovery_in_campaign` constraint and the path forms it
used instead of the per-constructor constraints and written-out joins.
The schema is left as written; this is noted for a future release, not
done here.
