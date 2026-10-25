---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 615
summary: unique index inserts do not check the latest committed state as mvcc.md §7 requires
branch: adtb-ssi
depends-on:
  - blink-tree
  - slotted-heap
  - commit-log-snapshots
tags:
  - engine
---

# Feature: uniqueness checks under concurrency

## Report

Implement the uniqueness check of `docs/design/mvcc.md` §7: an insert
into a unique index probes its key range and decides conflict, wait or
serialization failure by the table there, against the latest committed
state.

Prerequisites: `blink-tree.md`, `slotted-heap.md`,
`commit-log-snapshots.md`.
