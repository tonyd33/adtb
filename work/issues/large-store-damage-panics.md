---
created-at: 2026-11-09
status: open
kind: bug
priority: 300
summary: "two large-value store paths panic on a damaged page instead of refusing it: insert's tail update assert and sweep's not-a-heap-page panic"
tags:
  - engine
  - class-failure-flattened
---

# Bug: Large-value store panics on two damaged-page paths

## Observed

In crates/adtb/src/heap/large.rs, `insert`'s tail update asserts `written` and `sweep` panics with "touched store page .. is not a heap page". Both are reachable only on damage: the tail chunk is a head of count 0 written by the in-progress writer, and the store frees pages only through `free_value`.

## Expected

Each refuses the page through the store's damage path, as `LargeStore::refused` and `Heap::refuse_format` do: the latch dropped or the stage abandoned before damage is recorded.

## Reproduction

Plant a foreign or unframed page at a store position the sweep touches, through the testing surface, and run a sweep. Found by the step 7 review of structure-version-owner.
