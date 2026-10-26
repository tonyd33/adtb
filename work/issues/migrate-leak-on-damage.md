---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 697
summary: "open after a crashed migration keeps its pages when verify is incomplete, and clears the marker so they are never freed"
closed: 2026-10-25
tags:
  - quality
  - engine
---

# Bug: a crashed migration's pages survive an incomplete verify

## Report

`Catalog::open` (crates/adtb/src/catalog/mod.rs:490-501), finding `root.changing` naming a transaction that did not commit, runs verify and frees the leaked pages only `if report.complete`, then clears `changing` regardless. A damaged page anywhere makes the report incomplete (verify/mod.rs:142), so the migration's pages stay allocated and, with the marker gone, no later open frees them. docs/design/schema-change.md states that after a crash during `migrate`, `open` frees every page the migration allocated if it did not commit; nothing reports the failure.

Fix: keep `changing` set and return an error naming the damage when the report is incomplete (or free what the uncommitted transaction allocated without a whole-file verify), and test it with a damaged page plus a crash mid-migrate.

## 2026-10-25: resolved

## 2026-10-25: resolution

`open` fails with the damage and keeps the `changing` marker, so a later open over the mended page frees the pages.
