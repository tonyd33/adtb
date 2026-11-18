---
created-at: 2026-11-17
status: open
kind: feature
priority: 600
summary: "an index over a set-valued expression, one entry per element as interop.md 11.2 states and full-text search builds on, is not implemented in the engine"
tags:
  - engine
---

# Feature: Multi entry index

## Request

## Proposal

## Open questions

## Request

interop.md §11.2 makes an index element of a set type one entry per element, and §11.3 builds full-text search on it (`index (words(body))`). The checker admits it (ruled 2026-11-17 with index-element-unkeyable-type-panics) and the reference answers it; the engine refuses it at migrate as unimplemented.

## Proposal

Index maintenance writes one entry per element (insert, delete, update diffing the old and new sets), probing and planning treat the index as multi-entry, uniqueness is not offered over it. A searching test over insert, update and delete against the reference.
