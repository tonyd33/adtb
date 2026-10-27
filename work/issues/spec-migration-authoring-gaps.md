---
created-at: 2026-10-26
status: open
kind: feature
priority: 822
summary: "a printed schema cannot be replayed as a first migration, add field cannot declare a delete action, and a split product cannot refer to itself"
tags:
  - spec
  - evolution
---

# Feature: Migration authoring gaps

## Report

Found by the campaign manager (`apps/campaign/docs/gaps.md`):
- Within one migration only `Key r` may name a relation added later (evolution.md §3.2). A sum cannot declare a constructor-typed reference to itself in the operator that adds its type, and a schema printed by the engine cannot be replayed as a first migration. The app's 0001 builds `Entity` up in steps.
- `add field` takes a `type`, and a delete action belongs to `field_type`, so no operator gives a new or existing reference field an `on delete` action.
- The spec does not say how the products of a split constructor refer to themselves.

## 2026-10-26: proposed rulings

- Extend the forward-reference ruling: within one migration, `Key r.C` and `Key r.(Set)` may name a relation added later when its type is added in the same migration; constructor sets resolve when the migration ends. A printed schema, each declaration prefixed by `add`, is then a valid first migration, and a test replays it.
- `add field` takes a `field_type`, so `on delete` may follow its type; `alter field T.C.f on delete ...` changes the action of an existing reference field, and its inverse restores the old action.
- In a split, a self-reference field of the split constructor is typed in each product to the product set that replaces it, unless the split names the target.

These go through the design space before the spec changes.
