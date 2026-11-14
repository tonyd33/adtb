---
created-at: 2026-11-13
status: resolved
closed: 2026-11-14
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

## Resolved

Landed as tumnlvro ef08e67e (3 rounds, full check 0, 3745 tests). `violations` decides a constraint or check existed by `CatConstraintId` (a missing id is `Broken::ConstraintId`), so a dropped and re-added one is validated. Pending constraint and check drops are keyed by id in memory and in the held rows (`Rot::PendingRow` on a row whose id names another object; older files read as corrupt). Adding a constraint or check under the name of one whose drop is pending, either kind, from this migration or an earlier one, is refused naming the drop and its holders ("adding check auth::no_blank is refused: pending-dropped constraint auth::no_blank, held by web-1.0, keeps its name until its drop takes effect"); evolution.md §6.4 states it. The kept schema must hold each pending drop's id (`Broken::KeptConstraint`). Scripts: oplog/readded-constraint-validated.test and four under bundles/. Filed beside: redefine-fn-skips-constraint-revalidation, write-violations-keyed-by-name, bundle-pinned-past-taken-drop-unenforced.
