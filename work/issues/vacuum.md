---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 520
summary: "no vacuum: dead versions are never pruned, index entries never cleaned, and no visibility map exists"
branch: adtb-vacuum
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

## 2026-10-25: from the heap

Vacuum also has to:
- reopen old-format pages that conversion left at free-space category
  0 (mvcc.md §12.4);
- remove empty pages from a heap's directory, which nothing does yet;
- find out-of-line chunks written for a version that aborted or
  crashed, which are garbage nothing refers to.

The visibility map (§11) is not built.
