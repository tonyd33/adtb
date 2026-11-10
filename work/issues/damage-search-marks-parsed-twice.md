---
created-at: 2026-11-10
status: open
kind: bug
priority: 850
summary: "the damage search decodes the marks page in two places, ledger::marks and world::durably_whole, each with its own constants and offsets"
tags:
  - quality
  - test
  - sim
  - class-copied-rules
---

# Bug: two decoders of the marks page in the damage search

## Report

`marks` (crates/adtb-sim/tests/storage/damage_search/ledger.rs:304, new) and `durably_whole` (crates/adtb-sim/tests/storage/damage_search/world.rs:494) both decode the marks page, each with its own `DATABASE = 1` and offsets. A format change fixed in one leaves the other reading wrongly.

Fix direction: `durably_whole` becomes `ledger::marks(&page).is_some_and(|m| m.contains(&Quarantine::Database))`.

Found in the quality review of `reviewed..main` at 7bf40103.
