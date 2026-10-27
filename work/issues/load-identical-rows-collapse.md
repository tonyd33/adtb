---
created-at: 2026-10-27
status: open
kind: bug
priority: 791
summary: "a load of two identical rows with an explicit key may insert one row and report two, instead of failing with KeyViolation"
tags:
  - quality
  - engine
  - test
---

# A load of two identical keyed rows may insert one and report two

## Report

Unverified: found by reading; the repro below is the test to write.

`Load::commit` (`crates/adtb/src/api/load.rs:113-150`) turns each staged group into ad hoc text, `insert T from (from x in load_rows_N select { f: x.f, ... })`. `Staged::param` (`load.rs:192-227`) adds a `load_ordinal` field so equal rows stay distinct in the parameter set, but the `select` drops it, so two rows equal in every field collapse to one value of the set being inserted. `docs/spec/embedding.md` section 9.2 says two staged rows with one key fail with `KeyViolation`; `LoadReport.rows` counts staged rows (`load.rs:129`), so it reports 2 either way.

The tests miss the case: `equal_rows_of_a_load_are_each_inserted` (`crates/adtb-sim/tests/loads.rs:172`) uses a generated key, and `two_staged_rows_with_one_key...` (`loads.rs:224`) uses rows whose `body` differs.

Repro: push `[I64(7), "a", "x"]` twice into `notes` with `id` given, commit, expect `KeyViolation` and no row inserted.

Fix: carry the ordinal through to the key check (or build the change set directly instead of generating text), and add the test.
