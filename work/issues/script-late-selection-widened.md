---
created-at: 2026-11-16
status: resolved
closed: 2026-11-16
kind: bug
priority: 300
summary: a migration script may write a key outside the constructor selection the migration ends with
tags:
  - lang
  - class-none
---

# Script late selection widened

## Report

Found by the review of committed-schema-type part 2 (2026-11-16), by a probe through `Source::accept`. Not live: the engine refuses every `script` (crates/adtb/src/catalog/migration.rs:884-885, "the engine does not run a `script` yet").

A field typed with a selection that resolves only at the migration's end (`Person { home: Key regions.Settled }`, with `add set Settled of regions = Town` later in the same migration) is taken as every constructor until then: crates/adtb-lang/src/schema/types.rs:416-432 `Err(_) if late => None`. A `script` before the `add set` accepts `insert people.Living { home: regions.key(1) }`; the same insert after it is refused "expected Key regions.Town, found Key regions". docs/spec/evolution.md §3.1 resolves selections at the migration's end and §4.14 checks a script against the final schema's constraints, so the earlier script writes a key the final schema does not admit and nothing rechecks it. Reads stay conservative (a match must cover every constructor).

Fix direction: a script that writes, or matches by constructor through, a field whose late selection has not resolved is refused, as `relation_of` refuses a key of an ahead relation. Ruled into committed-schema-type part 2b; the part 2 test pins today's acceptance with a FIXME.

## Resolved

Landed as urmsumlx d610a136 (committed-schema-type part 2b-m-2, 2 rounds, full check 0, 3850 tests), on the `KeyCtors::Late` variant of 2b-m-1. A script, a fill or a routing check that writes, matches by constructor, or gives any value where a late-selected key is expected is refused ("a script may not give Key regions as Key regions.Settled: it has a selection this migration resolves at its end"); a caller-supplied parameter type with a late selection is refused at `Prepared::naming` for every text entry. Each other consumer of `Late` is decided in the 2b-m-2 receipt (refused, rechecked at the migration's end, or committed only). Tests in evolve_tests (script write, match, argument, fill before and after the set), the server and in-process parameter tests, and translate's Declared and Ahead `Late` arms.
