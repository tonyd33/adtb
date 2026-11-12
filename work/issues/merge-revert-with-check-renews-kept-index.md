---
created-at: 2026-11-12
status: open
kind: bug
priority: 310
summary: "with a check on a merged constructor, reverting the merge gives a set-scoped index it kept a new id"
tags:
  - class-copied-rules
---

# A merge revert renews a kept index's id when a check is present

## Report

With `check x_set on ts.A` present, reverting `merge constructor T.(A | B) into M ...` changes the id of `index (z) in S` (`set S of ts = A | B`) from 1031 to 1041. The same merge without the check keeps 1031. The revert neither creates nor re-scopes that index, so catalog.md §1.2 keeps its id. Found by the law search of structure-lang-identity step 5a (`KNOWN` row Merge `[check on ts.A]`).

Direction: the inverse by id restores clauses it did not replace without re-deriving them (structure-lang-identity step 5). Remove the `KNOWN` row.

Defect class: Wildcards and copied rules.
