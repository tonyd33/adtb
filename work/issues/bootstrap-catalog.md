---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 701
summary: "the engine has no catalog: no system relations with fixed ids to open a database from"
branch: adtb-boot
depends-on:
  - slotted-heap
  - blink-tree
  - commit-log-snapshots
  - id-assignment-gaps
  - schema-catalog-ids
tags:
  - engine
---

# Feature: bootstrap catalog and system relations

## Report

Implement the subset of `docs/spec/catalog.md` §1, §2 and §4 needed to
bootstrap, and `docs/design/schema-change.md` §1: system relations
stored as heaps with their fixed ids, catalog relations, error sums.

Prerequisites: `slotted-heap.md`, `blink-tree.md`,
`commit-log-snapshots.md`, `id-assignment-gaps.md`,
`schema-catalog-ids.md`.

## 2026-10-25: commit log root

The catalog bootstrap records the page id of the commit log's root
(`docs/design/mvcc.md` §4.2). Until then, callers keep it themselves.
