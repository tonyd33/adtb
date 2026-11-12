---
created-at: 2026-11-12
status: open
kind: bug
priority: 250
summary: "reverting an add set made where the source does not build is refused with a reason evolution.md does not list"
tags:
  - class-none
---

# A revert of add set inside a non-building run gives an unlisted reason

## Report

One migration `drop relation vs`, `add set S9 of ts = A`, `drop type V` (the source between the first and last does not build, since `V` names `Key vs`), then its revert: refused with "the log does not hold the object the operator names". evolution.md §5.4 lists no such reason; the revert should apply (§5.1) or be refused with a listed one. Same on main before structure-lang-identity step 5c-1. Found in the review of 5c-1.

Direction: the `add set` inverse reads the set's id from a source that builds, or the run's id map supplies it (structure-lang-identity step 5c). Test: the case above in the law search.

Defect class: none yet.
