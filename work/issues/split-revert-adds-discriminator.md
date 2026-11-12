---
created-at: 2026-11-12
status: open
kind: bug
priority: 330
summary: "reverting a split restores a unique over the split constructor with the discriminator added, so the revert changes the constraint"
tags:
  - class-copied-rules
---

# A split's revert adds the discriminator to a unique

## Report

Reverting `split constructor T.A into A1 { .. } | A2 { .. }` restores `unique (z) in A` as `unique (q, z) in A`: the inverse merge prepends its discriminator to every unique it rewrites, so the revert leaves a different constraint than the one the split replaced. Found by the step 3b search of structure-lang-identity (clause texts compared after each revert); pre-existing in the hand-written inverse.

Direction: an inverse computed by id restores the before-clauses as they were (structure-lang-identity step 5). Test: split then revert a relation with `unique (z) in A`; the clause text and id come back unchanged.

Defect class: Wildcards and copied rules.
