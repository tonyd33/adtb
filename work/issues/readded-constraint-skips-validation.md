---
created-at: 2026-11-13
status: open
kind: bug
priority: 300
summary: "a constraint dropped and added again under its name in one migration is not validated against existing rows, so a stricter body is admitted over rows that violate it"
tags:
  - engine
  - quality
  - class-identity-by-spelling
---

# A constraint dropped and added again in one migration is not validated

## Report

`Catalog::violations` (crates/adtb/src/catalog/validate.rs:24) skips a constraint or check when `existed.contains(&name) && !touched`. `existed` is the set of `module::name` strings of the old schema (validate.rs:34). A constraint dropped and added again under the same name in one migration is a new object with a new catalog id, and its body may differ. Because the name existed, it is skipped unless it reads a relation the migration rewrote. evolution.md requires an added constraint to hold over the existing rows when the migration applies.

`Catalog::denied`, just below (validate.rs:72), already pairs `constraint_ids()` and `check_ids()` with the normal terms. `violations` was changed in this range (c10fb37c) but kept the name set.

Repro (reproduced as a temporary engine script under crates/adtb-sim/scripts/oplog/):

```
seed 18
use shop
use customers

migrate
  module shop
  add constraint nobody =
    from c in customers
    where c.name == "zzz"
    select c

migrate refused MigrationRefused { version: 3, reason: Rejected { reason: SchemaRefused { operator: "", detail: "constraint shop::nobody is violated by Customer { id: customers.key(2), name: \"bob\" }" } } }
  module shop
  drop constraint nobody
  add constraint nobody =
    from c in customers
    where c.name == "bob"
    select c
```

Expected: the second migration is refused, as `add constraint` of a fresh name over the same body is in oplog/added-constraint-violated.test. Actual: it applies (`the open succeeds`), and `bob` violates a committed constraint.

A `redefine fn` that changes a function a constraint calls is skipped by the same test, since neither the name nor the read relations change.

Fix direction: decide "existed" by `CatConstraintId`, pairing ids with terms as `denied` does, and validate a constraint whose id `old` lacks. Also validate one whose body, or a function it reaches, changed.

Defect class: class-identity-by-spelling
