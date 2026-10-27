---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 690
summary: "the campaign manager's release 1.2 and 1.3 logs do not open in the engine: the split products' parent reference has no index over the new constructor set"
closed: 2026-10-26
tags:
  - quality
  - engine
---

# The campaign manager's release 1.2 log is refused by the engine at open

## Report

The real release 1.2 log, and so the real 1.3 log (`campaign::MIGRATIONS[..3]` and `[..4]`), does not open in the engine, fresh or over release 1.1 rows. Opening it fails with:

```
MigrationRefused { version: 3, SchemaRefused: "constraint campaign::location_parent_in_campaign is not admitted: changes to campaign::entities.(Region | Settlement | Site): index (parent) in campaign::entities.(Region | Settlement | Site) is missing" }
```

The split in `apps/campaign/schema/migrations/0003.adt:1-13` leaves the constraint `location_parent_in_campaign` (`apps/campaign/schema/campaign.adt:155`) needing an index on `parent` over the split products, and nothing creates one.

Nothing catches it. `apps/campaign/tests/front_end.rs` folds the log in adtb-lang only. `apps/campaign/tests/engine.rs` never opens `migrations(3)` or `(4)`: `release_1_3_shelves_a_quest_over_release_1_1` (engine.rs:532) applies 0004 over 0002 and skips 0003. `apps/campaign/docs/gaps.md:141` and `:282` mark the split gap resolved with no failing test, and `apps/campaign/docs/design.md:486` is stale either way.

Repro: a test in `apps/campaign/tests/engine.rs` that calls `adtb::open` on a scratch path with `Log::new(campaign::MIGRATIONS[..3])`.

Fix: decide where the index comes from. Either the split carries an implied reference index on `parent` over the new constructor set into the catalog, or 0003 declares `index (parent) in` the products; the spec settles which. Add engine tests that open the full 1.2 and 1.3 logs over 1.1 rows (locations with parents, NPCs with locations) and check the routed constructors, drop the 0002 plus 0004 workaround in engine.rs and `release_1_3_applies_over_release_1_1` in front_end.rs, and correct gaps.md and design.md.

## 2026-10-26: done

The spec settles it in the engine: each product holds its own `parent` with its own implied index (`index (parent) in Region`, and so on, per catalog.md §2.2), and constraints.md §5.1 asks only that every read go through index probes. The bounded planner looked for one index covering the whole constructor set. `Planner::join` (`crates/adtb/src/plan/lower/mod.rs`) now reads a constructor set that no one index covers as a fork of one read per constructor when each has a probe of its own; `docs/design/execution.md` §1.1 states it as a postcondition of `plan_bounded`. No catalog or migration change: a fresh schema with the products declared directly was refused the same way, and is now admitted.

Tests: `a_join_on_a_field_each_constructor_holds_is_admitted_through_their_implied_indexes` and `a_join_on_a_field_one_constructor_has_no_index_on_is_refused_naming_the_set` in `crates/adtb-sim/tests/constraints.rs`; in `apps/campaign/tests/engine.rs`, `release_1_2_opens`, `release_1_3_opens`, `release_1_2_routes_locations_by_scale_over_release_1_1`, `release_1_3_routes_locations_by_scale_over_release_1_1` and `a_split_location_and_its_parent_stay_in_one_campaign`, which runs the forked check on an insert and on moving a parent. `release_1_3_shelves_a_quest_over_release_1_1` opens the full 1.3 log over 1.1 rows; `release_1_3_applies_over_release_1_1` is gone from `front_end.rs`. `gaps.md` and `design.md` are corrected.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the cat track.
