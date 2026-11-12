---
created-at: 2026-11-12
status: open
kind: bug
priority: 330
summary: "reverting a merge over a clause scoped to some of the merged constructors restores it three times with three new ids"
tags:
  - class-copied-rules
---

# A merge revert restates a listed-scope clause per product and whole

## Report

`type T { id: Key ts, n: Text } = A { z: Text } | B { z: Text } | C { z: Text }` with `index (n) in (A | B)`; merge `T.(A | B)` into `M` gives `index (n) in M` (new id, right); `revert` gives `index (n) in A`, `index (n) in B` and `index (n) in (A | B)`, ids 1027, 1028, 1029. Same for `index (z) in (A | B)` and for `(B | A)` beside a set clause. Pre-existing on main. Cause: `before_clauses` (evolve/revert/mod.rs) states the original clause as a replacement while the inverse split's one-constructor branch (evolve/ctor/split.rs) also adds one `split_images` copy per product; the dedup matches identical text only.

Direction: an inverse computed by id restores the before-clauses and adds none of its own (structure-lang-identity step 5); until then, skip the split's per-product images for clauses the inverse supplies. Test: the scenario restores `index (n) in (A | B)` once.

Defect class: Wildcards and copied rules.
