---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 696
summary: "the differential sweep counts a query both sides refuse alike as a disagreement, so seeds picking the file scan conformance script fail"
closed: 2026-10-26
tags:
  - quality
  - test
---

# The differential corpus sweep fails when the engine and the reference refuse a query alike

## Report

`Got::agrees` (`crates/adtb-testkit/tests/differential.rs:89-97`) returns false whenever either side is `Refused`, so a query both sides refuse for the same reason counts as a disagreement. `Script::read` (`crates/adtb-testkit/src/corpus.rs:96-102`) keeps every query that type-checks, including the `read_csv` and `read_ndjson` queries of `conformance/interop/03-file-scans.test`. A corpus case gives the reference an empty in-memory file set and the engine no file root, so both refuse with `BadInput`.

```
ADTB_DIFF_SEED=136 ADTB_DIFF_CASES=1 cargo test -p adtb-testkit --test differential corpus_mutants_equal_reference
```

Actual:

```
the engine gives Refused(Rejected { reason: BadInput { source: "data/contacts.csv", line: 0, ... No such file or directory (os error 2) }), the reference Refused(Reject(BadInput { source: "data/contacts.csv", line: 0, detail: "data/contacts.csv: no such file" }))
```

Expected: the case passes or is skipped. Seed 225 picks the same script. The default seeds (0 to 12) and deep seeds (0 to 96) never do.

The shrinker (`shrink_query` and `shrink`, differential.rs:303-363) keeps any failure, so a shrink can also drift onto a query both sides refuse.

Fix: have `agrees` treat equal refusal reasons as agreement (or a refusal by both as not applicable), keep shrinking only toward the original kind of failure, and either drop file scanning queries in `Script::read` or give both sides the script's directory as the file root.

## 2026-10-26: done

`Got` in `crates/adtb-testkit/tests/differential.rs` has a `Rejected(Reason)` arm: the rejection reasons the engine and the reference both give, by their identifying fields (constraint, relation and key, ctor, message, source and line), with the free-text details left out. Equal reasons agree. Failures are a `Failure` (disagrees or panics) and both shrink loops keep only candidates failing the same way. `corpus.rs` `EXCLUDED` now drops queries naming `read_csv`, `read_ndjson`, `read_parquet` or `read_arrow`, whose files a case does not hold. Test: `a_rejection_both_sides_give_alike_is_agreement`. Seeds 136 and 225 pass.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the fbug track.
