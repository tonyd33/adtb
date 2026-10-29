---
created-at: 2026-10-28
status: in-progress
kind: bug
priority: 831
summary: "the permission provider renormalizes the program per statement, over-charges carried data in widened anonymous sums, and charges Rejects an insert on every destination constructor"
branch: permission-fixes
depends-on:
  - typed-permission-requirements
tags:
  - lang
  - quality
---

# Permission provider follow-ups

## Report

Found by the independent review of the typed permission requirements provider (work/briefs/review-permission-requirements.md, local).

- permission/mod.rs: `norm::program` runs again for every analyzed statement.
- check/adhoc.rs `retag_anon` builds fields without transport marks, so carried data in a widened anonymous sum is charged as read.
- permission/eval.rs: Rejects charges Insert on every constructor of the destination type, not only the constructors it can insert.

## 2026-10-28: from the review of the review fixes

From work/briefs/review-permission-fixes.md (local):
- permission/value.rs `each_fieldless` cannot fire for a checked schema, since every relation has one key shared by all its constructors. Drop it, and the synthetic test `consuming_a_stored_row_ranges_its_fieldless_constructors` that drives the Analyzer directly.
- Add `top` over `left union right` to the two-relation test, and B2 variants through a fix source and through an upsert's `old`.
- The `each_distinguishing_read` doc comment says "two such values" with nothing to refer to; say "two elements of a set".
- Over-charges, never under-charges: rows an `at` reads inside a fix body or a multi-constructor For body lose their key-distinct mark, and `prior(r)` outside a constraint is treated as another state.
