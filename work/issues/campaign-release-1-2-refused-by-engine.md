---
created-at: 2026-10-26
status: open
kind: bug
priority: 690
summary: "the campaign manager's release 1.2 and 1.3 logs do not open in the engine: the split products' parent reference has no index over the new constructor set"
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
