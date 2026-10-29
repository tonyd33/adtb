---
created-at: 2026-10-29
status: open
kind: bug
priority: 897
summary: "the pinned law credits every query of a step to every operator, never flags an operator no walk draws, and compares empty contents when listing a relation fails"
tags:
  - quality
  - test
---

# The pinned law's coverage check can pass vacuously

## Report

In `crates/adtb-testkit/tests/differential.rs`:
- `pinned_law` calls `translated(&step.operators, queries)`, crediting every query to every operator of the step whether or not the query reaches what the operator changes.
- `schema_operators_keep_their_laws` flags only operators recorded with a count of 0; an operator in `TRANSLATES` that no walk draws is never recorded and so never flagged.
- `contents` uses `unwrap_or_default()` when listing a relation fails, so both sides are empty and the law passes having compared nothing.

Fix: count a query for an operator only when the operator touches a type or relation the query reaches (`Schema::reached`); flag `TRANSLATES` entries absent from the counts; fail the case when `contents` cannot list a relation.
