---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 590
summary: "catalog sim tests check how many reasons a refusal gives, not which"
branch: adtb-qtest
tags:
  - quality
  - sim
  - test
---

# Catalog refusal tests pass for any refusal with the right number of reasons

## Report

`only_a_first_migration_of_declarations_applies` (`crates/adtb-sim/tests/catalog.rs:576`) checks `1 == why.len()` (`:586`, `:596`) and `!why.is_empty()` (`:589`), and `a_function_naming_a_wasm_module_no_migration_installs_is_refused` checks `1 == why.len()` (`:664`). A refusal for an unrelated reason passes each. `a_crash_at_every_flush_during_the_first_migration_reopens_to_all_or_none_of_it` (`:508`) checks only that the first crash point opens at version 0 and the last at 1 (`:525`, `:526`), not that the versions never go from 1 back to 0 between flushes.

Fix: assert the text or variant of each refusal reason; assert the flush versions are sorted.
