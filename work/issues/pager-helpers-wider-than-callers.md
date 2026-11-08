---
created-at: 2026-11-08
status: resolved
kind: bug
priority: 890
summary: "five pager.rs helpers that bypass the owners are pub(crate) with callers only in storage/, and load_marks is pub with callers only in the crate"
closed: 2026-11-08
tags:
  - quality
  - engine
---

# Pager helpers that bypass the storage owners are visible to the whole crate

## Report

crates/adtb/src/storage/pager.rs exposes helpers that bypass the storage owners wider than their callers need:

- `unreserve` (pager.rs:1225), `file_end` (1237), `read_disk` (1246), `make_room` (1262) and `write_out` (1312) are `pub(crate)`, with callers only in crates/adtb/src/storage/. Any module of the crate can change the clock's resident list or write a page's bytes without its latch.
- `#[doc(hidden)] pub fn load_marks` (crates/adtb/src/storage/quarantine.rs:378) has callers only in the crate (recovery.rs:115, testing/plant.rs:150).

Fix direction: the five helpers `pub(super)`, `load_marks` `pub(crate)`. `Pager::damage` (pager.rs:1188) is also `#[doc(hidden)] pub`, but adtb-sim's backup.rs and quarantine.rs call it, so it narrows when T9's remaining splits move those tests.

## 2026-11-08: resolved

Storage owners step 8: `unreserve`, `file_end`, `read_disk`, `make_room` and `write_out` are `pub(super)`, `load_marks` and `Pager::damage` `pub(crate)`, their `#[doc(hidden)]` gone; no caller outside the crate remains.
