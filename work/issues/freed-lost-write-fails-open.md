---
created-at: 2026-11-07
status: open
kind: bug
priority: 135
summary: "after a crash, a lost write of a heap or directory page's freed incarnation fails the open with Corrupt Stale found Lsn(0) instead of opening and reporting the page"
tags:
  - class-failure-flattened
---

# A lost write of a freed incarnation fails the open after a crash

## Report

## 2026-11-07

Found by the damage class search (damage_search in crates/adtb-sim/tests/storage). A lost write of a page's freed incarnation, followed by a crash, fails the next open with `Corrupt Stale { found: Lsn(0) }`. Heap and directory pages both show it.

- Seed 3 write 641 (directory): Insert users, Insert docs, Reopen, Update users, Migrate (add tags), Checkpoint, Migrate (drop tags), Crash, Checkpoint.
- Seed 6 write 684 (heap).

storage.md §6 and §8: a stale page is refused on the reads that reach it and reported, and the open fails only for the whole-database causes §8 lists. One page's lost write of an incarnation the database has since freed is none of them. A blank page with an expected LSN is stale with LSN 0, so the open is likely judging a freed page by its earlier incarnation's evidence, or treating a stale page found in recovery as fatal.

Direction: find which check fails the open, and whether a freed incarnation's expected LSN still reaches recovery. The search gates this cell in `KNOWN`; the fix removes the gate entry and its pinned cases must then pass.

Defect class: Damage or failure gives a panic, hang or flattened error.

## 2026-11-07, round 2

The search also files a pair under this ticket: lost write 644 (LostEarlier Directory of tags) then torn write 645, seed 3. Verify and scrub name page 570 `Stale { found: Lsn(0) }` while the file holds the next incarnation. Same cause suspected: the freed incarnation's evidence judges the next one. Split it out if the fix shows another cause.
