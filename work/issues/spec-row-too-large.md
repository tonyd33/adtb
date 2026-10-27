---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 295
summary: the spec states no limit and no error for a row too large to store
closed: 2026-10-26
tags:
  - spec
  - design
---

# Bug: No limit or error for a row too large to store

## Report

The heap refuses a row whose fixed part does not fit on a page, with
`Error::TooLarge`. No error sum in `docs/spec/catalog.md` §4 carries
it, and no part of the spec states a limit. Two concrete schemas:

- a record of 300 `Decimal` fields, whose fixed part exceeds a 4 KiB
  page;
- a record of 400 `Text` fields, whose out-of-line pointers alone
  exceed a page.

The limit depends on the page size, which the database chooses at
creation. So a static refusal would make a schema valid in one
database and not in another, while a runtime fault would let a checked
schema fail every insert. Open a design-space question, then rule.
Candidates:

- a field limit stated independently of page size, small enough for
  the smallest page;
- a refusal at migration time against the open database's page size;
- rows that continue across pages.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed in vwztzyqp, ruling `spec/row-too-large/refuse-at-write`. writes.md §1.6, semantics.md §6.6, evolution.md §3.2 and catalog.md §4.2 state `RowTooLarge { relation, ctor, size, limit }`, constructor id 467, with the catalog field ids after it renumbered inside the unreleased 2026-10 edition. The engine refuses the statement before any large-value chunk is written, and refuses a migration whose constructor can never fit a page. The reference takes a caller-supplied row measure.
