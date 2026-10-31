---
created-at: 2026-10-31
status: open
kind: bug
priority: 399
summary: "RemoveConstructor, DropUnique and DropCheck never reach the engine in walks, and do not yet apply as their laws state"
---

# Staged operators engine

## Report

`STAGED` in adtb-testkit/tests/differential.rs is skipped by the walks that run. `every_schema_walk_keeps_engine_equal_to_reference` and `every_operator_keeps_its_laws` draw these operators but are `#[ignore]`, with the reason "draws operators the engine does not yet apply as their laws state", and nothing runs them. operator-law-follow-ups is resolved and left this. Run the ignored tests, list what fails, then fix the engine and remove `STAGED`.
