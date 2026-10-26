---
created-at: 2026-10-25
status: open
kind: feature
priority: 702
summary: "the spec gives no catalog ids to several engine-defined and re-scoped objects, and three operators carry only their first id"
tags:
  - spec
---

# Feature: close the catalog id assignment gaps

## Report

`docs/spec/catalog.md` §1.2 fixes the id assignment order, but leaves
these without rules:

- engine-defined objects other than types and constructors have no ids
  below 1024;
- indexes re-scoped by split, merge or nest;
- `move field`;
- the result type of a stored view;
- `install` packages;
- `AddRelation`, `AddType` and `Store` carry only their first id.

Each needs a ruling in the design space before the spec changes.

## 2026-10-25: questions raised

Six questions under `spec/ids/` in the design space: `engine-objects`, `rescoped-index`, `moved-field`, `view-type`, `package`, `operator-ids`.

## 2026-10-25: ruled

Ruled in proposed point `ids1`:
- the engine's fields, indexes and other objects take a fixed table in
  `catalog.md` §4.6, below 1024 per id space, per catalog version;
- an index re-scoped by split, merge or nest is a new index with a new
  id;
- a moved field keeps its id, and a move down gives each further
  constructor a new field. The checker gives a moved field a new id
  today; change it;
- `store f` creates a one-constructor type for the view's element type,
  after the view's relation variable;
- an installed package's declarations take ids as operators of the
  installing migration, in package order;
- an operator records only its first id; the rest follow from §1.2's
  order and the decl term.

Write these into `docs/spec/catalog.md` §1.2 and §4.6 and make the
checker follow them.
