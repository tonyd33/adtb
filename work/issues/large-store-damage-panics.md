---
created-at: 2026-11-09
status: resolved
closed: 2026-11-17
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

## Resolution 2026-11-17

Fixed by `fix: report slot and chunk framing that does not decode as damage of its page`, part 2 of page-decode-returns-result: insert's tail check refuses through the stage and sweep refuses a page that is not a store page; both tested in crates/adtb/src/heap/row_damage_tests.rs.
