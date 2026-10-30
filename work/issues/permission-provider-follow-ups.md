---
created-at: 2026-10-28
status: open
kind: bug
priority: 1032
summary: the permission provider over-charges rows an at reads inside a fix or For body
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

## 2026-10-29: landed with the permission analyzer fixes

Landed with permission-keyof-join-panic and permission-change-path-blowup after a clear fresh review; branch check on 4f266022 passed (root 2710 passed, 7 skipped; campaign 47). `norm::program` runs once per analyzed program; `retag_anon` builds transport fields; `each_fieldless` and its synthetic test are removed; the doc comment is fixed; the B2 variants are added. Not possible: `top` over `left union right` is refused by the checker as indeterminate.

Remaining:
- crates/adtb-lang/src/permission/eval.rs: `REJECTED_ROW = 0` picks the `RejectedRow` constructor by position; base's loop over `ctors()` already charged exactly one constructor. Revert to the loop. `rejects_requires_insert_on_its_one_constructor` filters to inserts instead of asserting an exact set.
- Over-charges: rows an `at` reads inside a fix body or a multi-constructor For body lose their key-distinct mark (keeping it needs analysis of what the point depends on); `prior(r)` outside a constraint is treated as another state.

## 2026-10-29: rejects charge reverted to the constructor loop

Landed 7b4c5ee5 after a lead review; branch check on 89eac469 passed (root 2726 passed, 7 skipped; campaign 47). `REJECTED_ROW` is gone and `rejects` charges an insert on each constructor of the destination row type; `rejects_requires_insert_on_its_one_constructor` asserts the exact set, with the near miss `insert_without_rejects_requires_no_destination_insert`.

Remaining, over-charges only:
- Rows an `at` reads inside a fix body (aggregate.rs `fix`) or a For body (eval.rs `for_each`) lose their key-distinct mark. Keeping it needs proof that the point does not depend on the bound variable or the fix binders.
- `prior(r)` outside a constraint is unreachable: the checker refuses it there and no permission entry point analyzes a constraint. Nothing to fix until one does.
