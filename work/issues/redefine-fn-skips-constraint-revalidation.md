---
created-at: 2026-11-14
status: open
kind: bug
priority: 310
summary: "a redefine fn that changes a function a constraint calls does not revalidate the constraint over existing rows"
tags:
  - engine
  - class-identity-by-spelling
---

# A redefined function leaves the constraints that call it unvalidated

## Report

`Catalog::violations` (crates/adtb/src/catalog/validate.rs) validates at a migration only a constraint or check that is new by catalog id or that reads a relation the migration rewrote. A `redefine fn` that changes the body of a function a kept constraint calls changes what the constraint means, yet neither its id nor its read relations change, so it is skipped and existing rows may violate it. Found in the review of readded-constraint-skips-validation; not reproduced.

Direction: validate a kept constraint or check whose term, after normalisation with the migration's functions, differs from the old one, or that reaches by id a function the migration redefined. Test: an engine script whose `redefine fn` makes an existing row violate a constraint calling it, expecting the migration refused.

Defect class: Catalog rows and identity by position or spelling.
