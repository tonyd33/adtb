---
created-at: 2026-10-27
status: open
kind: feature
priority: 880
summary: "the schema walk draws no rekey, named sets, query constraints, drop relation or drop type, and keeps staged operators out of the default sweeps"
tags:
  - test
---

# Feature: schema operator law follow-ups

## Report

- The walk generator in `crates/adtb-testkit/src/cases/walk.rs` does not draw rekey, named sets and set membership, query constraints, drop relation or drop type.
- Remove constructor, drop unique and drop check are drawn but kept out of the default sweeps by the `STAGED` skip list in `crates/adtb-testkit/tests/differential.rs`. They passed at the seeds tried; move them into the default sweeps.
- `add default` is in the `UNRESOLVED` list until `resolved-operators` gives it a resolved form; the ignored test `an_added_default_fills_a_later_insert_and_its_revert_drops_it` in `crates/adtb-sim/tests/oplog.rs` states it.
