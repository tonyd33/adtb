---
created-at: 2026-11-12
status: open
kind: bug
priority: 330
summary: "reverting a nest of two constructors widens a unique over one of them to both"
tags:
  - class-copied-rules
---

# A nest's revert widens a unique

## Report

Reverting `nest constructor T.(A | B) into N { inner: Inner }` restores `unique (n) in B` as `unique (n) in (A | B)`: the inverse unnest re-scopes the clause to every product, so the revert enforces a wider constraint than the one the nest replaced and can refuse rows the original schema accepted. Found by the step 3b search of structure-lang-identity; pre-existing in the hand-written inverse.

Direction: an inverse computed by id restores the before-clauses as they were (structure-lang-identity step 5). Test: nest then revert a relation with `unique (n) in B`; the clause text and id come back unchanged.

Defect class: Wildcards and copied rules.
