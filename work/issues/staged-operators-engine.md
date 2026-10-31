---
created-at: 2026-10-31
status: resolved
kind: bug
priority: 399
summary: "RemoveConstructor, DropUnique and DropCheck never reach the engine in walks, and do not yet apply as their laws state"
closed: 2026-10-31
---

# Staged operators engine

## Report

`STAGED` in adtb-testkit/tests/differential.rs is skipped by the walks that run. `every_schema_walk_keeps_engine_equal_to_reference` and `every_operator_keeps_its_laws` draw these operators but are `#[ignore]`, with the reason "draws operators the engine does not yet apply as their laws state", and nothing runs them. operator-law-follow-ups is resolved and left this. Run the ignored tests, list what fails, then fix the engine and remove `STAGED`.

## Resolved 2026-10-31

`STAGED` and the `#[ignore]`s are gone; `every_*` run in the deep profile. Fixed: reads of an emptied constructor set, `merge_inverse` text, pins widening across an add, merge and nest of all-deprecated sources per evolution/merge-deprecated/same-state, and the upsert binder `old`, typed by what its probe can return. Follow-ups: pinned-writes-past-pin, staged-operators-test-gaps.

## 2026-10-31: resolved
