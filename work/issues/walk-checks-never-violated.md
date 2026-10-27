---
created-at: 2026-10-27
status: open
kind: feature
priority: 897
summary: "the schema walk's checks bound by a value no drawn row goes below, so the checked-merge sweeps cannot tell a carried check from a dropped or over-applied one"
tags:
  - quality
  - test
---

# Feature: schema walk checks that some drawn row violates

## Report

Every walk check is `field >= INT_MIN` with `INT_MIN = -2`, the least integer a case draws (`crates/adtb-testkit/src/cases/world.rs:27`, `:623`); `add_check` (`crates/adtb-testkit/src/cases/walk.rs:1278`) and the walk's writes never produce a violating row. The new sweeps `merges_carrying_checks_keep_engine_equal_to_reference` and `merges_carrying_checks_keep_their_laws` (`crates/adtb-testkit/tests/differential.rs:1060`, `:1072`) therefore catch only a merge that refuses or a revert whose text differs; a merge that drops the carried check, or applies it to the other source's rows, still passes. The world model hides the second case: `world.rs:463-472` widens a check over one source to the whole merged constructor, where the engine writes `match disc | A -> body | B -> true`.

Fix: draw some violating writes on the checked constructor and on the other merged one, and model a carried check as scoped to its source's discriminator value.
