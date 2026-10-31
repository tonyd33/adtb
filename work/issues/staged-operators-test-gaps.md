---
created-at: 2026-10-31
status: open
kind: bug
priority: 402
summary: "Walks and operator tests leave gaps: drops compared by count, Conflict::Unique untested, Y-data mutant survives, deprecated-only merges never drawn"
---

# Staged operators test gaps

## Report

## Report

Left by staged-operators; nothing here is a known wrong answer.

- `constraints_differ` in crates/adtb-testkit/tests/differential.rs compares counts, so a mutant dropping the wrong constraint survives.
- The `Conflict::Unique` branch of `bind_probed` in translate/walk.rs has no test or mutant.
- Mutant `Y-data` survives: the `Data` narrowing in `Map::ty` is now a second guard behind `exec::types::field`. Give it a test or remove it.
- Walks never draw a merge of only deprecated sources. Named tests cover it, and the inverse of a split at seeds 27, 33, 44, 57, 260.
- Refusal tests of deprecated merges use a substring. The `mixed` one omits the constructor.
- The operator sweeps run 1 seed at the default profile. Three take 5 to 6 s.
