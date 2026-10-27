---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 761
summary: "the differential tests found no disagreement over 282 seeds, with no check that they can find one, no floats, rare fix, no shrinking, and no compiled versus interpreted toggle"
closed: 2026-10-26
---

# Differential strength

## Report

## Report

`crates/adtb-testkit/tests/differential.rs` found no disagreement over 282 seeds. Show the tests can fail: break one engine rewrite or operator by hand in a scratch copy and confirm a seed finds it, and assert a floor on the share of queries with a non-empty result. Then add floats under the tolerance rule of `docs/design/oracle.md`, more `fix` (semi-naive gets little exercise), shrinking of a failing case, and the corpus-mutation and schema-walk methods. `docs/design/oracle.md` 5.5 lists a compiled versus interpreted toggle that `exec` has no switch for. Engine open costs about 1.5 s in debug on an empty log and 1.4 s more per small migration, which caps seeds per run.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed in psmvtrym. Three mutations of the engine (a planner rewrite, a hash-join build, constructor routing) each fail the differential tests; the old generator missed the first two. The generator draws up to three generators per comprehension, joins, point equalities on loaded values, floats with a tolerance, `fix` and `match`; floors assert the shares of each form. Failures shrink. Corpus mutation and schema walks are new. The stronger tests found a planner bug, fixed in the same change: an index probe dropped a second equality on the same element as a condition.
