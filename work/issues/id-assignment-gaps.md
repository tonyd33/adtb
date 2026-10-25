---
created-at: 2026-10-25
status: open
kind: feature
priority: 185
summary: "the spec gives no catalog ids to several engine-defined and re-scoped objects, and three operators carry only their first id"
tags: [spec]
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
