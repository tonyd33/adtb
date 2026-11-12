---
created-at: 2026-11-12
status: open
kind: bug
priority: 300
summary: "a unique clause scoped to one constructor leaks its scope into a later computed clause's match"
tags:
  - class-none
---

# A `unique ... in C` clause leaks into a later computed clause

## Report

`add relation zys : Zy key id generate sequence unique (a) in Z1 computed zzk: Text = match self | Z1 p -> p.a | Z2 p -> p.b` is refused with "`Z2` is not among the constructors of Zy.Z1" and "no `zys` in scope". `add computed ts.zzk ... match self` over a relation with such a unique is refused the same way. A computed field ranges over the whole relation (its `self` is any row). Found in the delta review of structure-lang-identity step 5c-2.

Direction: the scope of a clause applies to that clause only; find where the checker carries it. Test: both texts check.

Defect class: none yet.
