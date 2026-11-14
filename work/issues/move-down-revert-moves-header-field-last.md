---
created-at: 2026-11-12
status: resolved
kind: bug
priority: 300
summary: "reverting a move of a header field down restores the field at the end of the header, changing the type's field order"
closed: 2026-11-13
tags:
  - class-copied-rules
---

# A move-down revert puts the header field last

## Report

On header `{ id, n, o, h }`, `move field T.n to T.(A | B)` reverted with `fill T.A.n = match | C c -> ""` gives `{ id, o, h, n }`. Field order is part of the type. `Inverse::Reorder` exists for constructor fields; the move-down inverse does not reorder the header. A `KNOWN` row of the law search `every_invertible_operator_and_its_revert_restore_the_source_before_it` (evolve_law_tests.rs) pins it.

Direction: the inverse restores the header field at its old position (structure-lang-identity step 5, inverses by id). Remove the `KNOWN` row.

Defect class: Wildcards and copied rules.

## 2026-11-13: resolved
