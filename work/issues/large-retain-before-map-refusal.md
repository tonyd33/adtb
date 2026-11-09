---
created-at: 2026-11-09
status: open
kind: bug
priority: 150
summary: "HeapWrite::commit raises large-value counts in updates of their own before clear_written, so a visibility-map refusal there fails the write with the counts left raised and the values never freed"
tags:
  - engine
  - class-not-atomic
---

# Bug: A failed heap write can leave large-value counts raised

## Observed

`HeapWrite::commit` (crates/adtb/src/heap/write.rs) commits `retain_apart` before `clear_written`. When `clear_bit` meets a misshaped map page, `Vm::refuse` abandons the map stage and the write fails, but the retains are already committed. Scenario: an update whose `held` or gained values are non-empty, on an all-visible page, with a damaged inner map page. Each retained value keeps a count of one more than the versions naming it, so the store's sweep never frees it.

## Expected

A write that fails commits nothing that a later sweep cannot undo: either `clear_written` runs before the retains, or a failed write releases what it retained. mvcc.md §12.5 states that counts rise in updates of their own before the version's update; the fix keeps that order for the version's update.

## Reproduction

Plant a damaged inner visibility-map page under a heap with an all-visible page holding a version with an out-of-line value, update that version, and read the value's count after the refusal. Found by the step 7 delta review of structure-version-owner; reasoned, not run.
