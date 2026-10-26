---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 530
summary: "verify does not exist, so a heap or tree corrupted by a crash goes unnoticed"
branch: adtb-verify
depends-on:
  - slotted-heap
  - blink-tree
tags:
  - engine
  - test
---

# Feature: verify structural checks

## Report

Implement the structural checks of `verify` over heaps and trees: page
checksums and kinds, slot and chain integrity, tree ordering and
sibling links, every index entry naming a live slot. Milestone 5 exits
with `verify` clean after every crash. The cross and logical checks of
`docs/spec/catalog.md` §11 are milestone 9.

Prerequisites: `slotted-heap.md`, `blink-tree.md`.

## 2026-10-25: leaked pages

The commit log can leak pages on a crash: between `allocate` and the
update that links the new page (create and reserve), or between the
root update and the frees (truncate). Heaps and indexes will have the
same window. `verify` should report every allocated page that nothing
refers to, and a repair should free it.
