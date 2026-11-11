---
created-at: 2026-11-11
status: open
kind: bug
priority: 470
summary: "an index entry whose constructor id the index does not map panics the process in Store::entries, where fetch_past skips such an entry"
tags:
  - quality
  - engine
  - class-failure-flattened
---

# An index entry naming a constructor the index does not map panics the process

## Report

`Store::entries` (crates/adtb/src/exec/store.rs:598-600) maps each entry's constructor id to a heap with `unwrap_or_else(|| panic!("index {id:?} maps constructor ..."))`. `Store::fetch_past` (store.rs:765-767) meets the same case and skips the entry with `continue`. Two reads of one index disagree on whether the case is possible.

An entry whose constructor id the index does not map is index damage with a sound checksum (for example a stale index page from before a migration dropped a constructor, read while the read-in gate is off). Damage ends in a refusal of the page, not a panic. Found by reading; not reproduced, and the code predates the reviewed range.

Direction: refuse the index page through the index damage path (`Rot::Index`, as an entry that does not decode is refused) in both reads; a test planting an entry with an unmapped constructor id and expecting the index refused and marked.

Defect class: Damage or failure gives a panic, hang or flattened error.
