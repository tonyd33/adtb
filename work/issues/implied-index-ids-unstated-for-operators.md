---
created-at: 2026-11-14
status: open
kind: bug
priority: 520
summary: "catalog.md §1.2 states where implied indexes take ids only for add relation and move field, though rekey, drop index, drop unique and add field with a reference also create them"
tags:
  - spec
  - class-none
---

# Implied index ids unstated for most operators

## Report

catalog.md §1.2's table lists the implied indexes an operator creates only in the `add relation` and `move field` rows. `rekey`, `drop index`, `drop unique` and `add field` of a reference also create implied indexes (a probe: `rekey relation ps key end` created start's implied index at 1029). Found in the review of the prelude-ref-relation track.

Direction: widen those rows with the implied indexes each creates and their order, as the code does; spec-revision workflow.

Defect class: none (spec gap).
