---
created-at: 2026-10-26
status: open
kind: bug
priority: 761
summary: "the differential tests found no disagreement over 282 seeds, with no check that they can find one, no floats, rare fix, no shrinking, and no compiled versus interpreted toggle"
---

# Differential strength

## Report

## Report

`crates/adtb-testkit/tests/differential.rs` found no disagreement over 282 seeds. Show the tests can fail: break one engine rewrite or operator by hand in a scratch copy and confirm a seed finds it, and assert a floor on the share of queries with a non-empty result. Then add floats under the tolerance rule of `docs/design/oracle.md`, more `fix` (semi-naive gets little exercise), shrinking of a failing case, and the corpus-mutation and schema-walk methods. `docs/design/oracle.md` 5.5 lists a compiled versus interpreted toggle that `exec` has no switch for. Engine open costs about 1.5 s in debug on an empty log and 1.4 s more per small migration, which caps seeds per run.
