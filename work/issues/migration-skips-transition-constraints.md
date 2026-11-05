---
created-at: 2026-11-04
status: open
kind: bug
priority: 440
summary: "add constraint and migration validation skip any constraint reading inserted, deleted or prior, so a mixed constraint's state part goes unchecked against existing data"
---

# Validation against existing data skips transition constraints

## Report

## Report

crates/adtb/src/catalog/validate.rs (near line 61) skips a constraint that reads `inserted`, `deleted` or `prior` when it checks existing data, as `add constraint` and `verify` do. constraints.md §6.2 says that against existing data the change is empty: `inserted` and `deleted` are empty and `prior(R)` is `R`. So a mixed constraint (`... where u.name == "x" or exists d in deleted(users) ...`) is admitted over rows its state part refuses, and a `prior(R)`-only constraint is never checked.

Found in the review of dump S5.3, 2026-11-04; restore's own pass is fixed in that track. Not reproduced for migrations.

## Proposal

Evaluate these constraints with the transition inputs bound empty and `prior(R)` bound to `R`, in `add constraint`, migration validation and `verify`, sharing restore's binding. Test each with a mixed and a prior-only constraint over violating rows.
