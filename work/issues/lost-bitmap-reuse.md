---
created-at: 2026-10-30
status: in-progress
branch: lost-bitmap
kind: bug
priority: 411
summary: "a lost or zeroed bitmap reads as free, so an allocation reuses named pages: rows vanish with no mark, and a bitmap zeroed in place overwrites live data"
---

# Lost bitmap reuse

## Report

`hold_free_page` reads a lost bitmap as blank. Found reviewing truncated-open; none of it comes from that change.

- A cut across a bitmap group, on a file over about 535 MB: no bytes overwritten, but one insert into another relation reformats a lost page. `select users` then answers `Ok(0)` with no mark, also after reopen.
- A bitmap zeroed in place: an allocated page is reused and the insert answers `Ok`, overwriting live data. A run of 40 inserts then did not finish in 60 s; cause not traced.

Also: a zero directory root then an insert panics `directory node overflows`, heap/dir.rs:59. A zero large-store page then a vacuum panics, heap/page.rs:105. A large-store page in a lost group answers `Fatal Storage` with no mark on every retry. Zero B-tree and visibility-map pages are not refused, and a write reaching a zero directory node before any read still reads it as empty through `Space::find`.

Direction: a blank bitmap page within a group that holds named pages is damage. Done when each case above refuses and marks, with a test.

## 2026-10-31: first round reviewed

Two blockers, both silent loss: open's `abort_unresolved` writes to a blank status page; recovery's checkpoint does not store the group count, so crash, reopen, crash, cut reuses a named page. Second round in the same workspace.
