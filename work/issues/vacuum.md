---
created-at: 2026-10-25
status: open
kind: feature
priority: 520
summary: "no vacuum: dead versions are never pruned, index entries never cleaned, and no visibility map exists"
depends-on:
  - slotted-heap
  - blink-tree
tags:
  - engine
---

# Feature: vacuum and the visibility map

## Report

Implement `docs/design/mvcc.md` §10.1, §10.2, §11 and §12.6: vacuum
over batched dead sets, heap pruning, index cleanup, and the
per-heap visibility map that vacuum sets and writes clear. The dead set
is an input here; the horizon that computes it is the watermark of
`commit-log-snapshots.md`.

Prerequisites: `slotted-heap.md`, `blink-tree.md`.
