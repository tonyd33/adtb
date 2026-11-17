---
created-at: 2026-11-15
status: resolved
closed: 2026-11-17
kind: bug
priority: 120
summary: "a redefine fn leaves an expression index over the function holding keys from the old body, so a unique is violated"
tags:
  - engine
  - class-identity-by-spelling
---

# Index keys stale after redefine fn

## Report

Found by the review of structure-lang-identity step 10c-2 and confirmed by a probe on its tree (sunvvrks). `change::shapes` (crates/adtb/src/catalog/change.rs) decides which relations a migration rewrites from each index's element text. `redefine fn` changes a function's body but not the text of an index element that calls it, so a relation with an expression index over the function is not rewritten and the index keeps keys computed with the old body.

Repro (engine script): `unique (twice(n)) in items` with `fn twice (x: I64) -> I64 = x * 2`; insert n = 10 and n = 15 (keys 20, 30); migrate `redefine fn twice (x: I64) -> I64 = x + 10` (keys now 20, 25). Insert n = 20 (key 30): refused, "constraint `app::items.unique#1025` is violated by 2 rows", a collision only with row 2's old-body key. Insert n = 15 (key 25): accepted, a duplicate of row 2 under the new body. The unique is violated in a committed state. A plain index lookup through `twice` plans a scan, so lookups do not show it.

Same cause as redefine-fn-skips-constraint-revalidation: a migration decides what a function change touches by the text that names the function, not by the function's identity and body. Fix direction: one dependency relation from each index element, constraint and check to the functions it reaches (by function id), and a redefine that changes a body marks every dependent index for rebuild and every dependent constraint for validation; a test that redefines a function under each kind of dependent.

Defect class: Catalog rows and identity by position or spelling.

## Resolution 2026-11-17

Fixed by `fix: rebuild and validate the dependents of a redefined function by id` (function-dependents-by-id): a changed body rewrites every relation whose index element reaches the function, so a key collision is refused and keys match the new body.
