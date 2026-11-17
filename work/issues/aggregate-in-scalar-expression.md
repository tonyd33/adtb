---
created-at: 2026-11-17
status: open
kind: bug
priority: 450
summary: "a user aggregate in an index element, row check or domain predicate is refused by the engine at migrate as an aggregate in a scalar expression; where the checker and the reference stand is not established"
tags:
  - engine
  - lang
  - class-none
---

# Aggregate in scalar expression

## Report

Found by fn-dependents (2026-11-17): a user aggregate in an index element, row check or domain predicate is refused by the engine at migrate ("an aggregate in a scalar expression"); only a constraint can call one. Not yet established whether the checker refuses it first or the reference accepts it.

Expected: one decision, stated in the spec, enforced once in the checker; the reference and the engine agree. Probe the three kinds on both sides first.
