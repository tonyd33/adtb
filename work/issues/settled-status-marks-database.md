---
created-at: 2026-11-07
status: resolved
kind: bug
priority: 136
summary: "a lost write of a commit log status page that the open settles marks the whole database and refuses every read, where storage.md says a settled status page marks nothing"
closed: 2026-11-08
tags:
  - class-not-atomic
---

# A settled status page marks the database

## Report

## 2026-11-07

Found by the damage class search, round 2 (gate `SETTLED_MARKS`). LostCurrent CommitLog, seed 3 write 637: Insert users, Insert docs, Reopen, Update users, Migrate, Checkpoint, Migrate, Crash, Checkpoint. Status page 13 is recorded Settled and reads fresh at the end, yet the database holds a `Database` mark and every read is refused "the database". Recycled CommitLog seed 35 write 641 shows the same in the deep tier.

storage.md §8: a status page the commit log reads stale, settles and writes back marks nothing.

Direction: find what sets the mark after the settle, whether the settle's own detection or a later check that judges the page by its pre-settle evidence. The default suite reruns seed 3 write 637 (`a_pinned_known_failure_still_shows`); the fix repoints it.

Defect class: Damage or failure gives a panic, hang or flattened error.

## 2026-11-08: resolved

Cause, traced on seed 3 write 637: the lost write is the open's own write-out of status page 13, and an ordinary read during the open's migration refuses and records the page `Stale`, failing the open. The failed open's mapper takes that record, and its walk (`Catalog::load` opening the commit log) settles the page, so the record is `Settled` by the time the walk returns; the mapper marked the database before checking the record was still current. Fix: the evidence stamps each change of a page's damage or incarnation with an epoch, `begin_mapping` hands out the epoch a mapping starts at, and the mapper marks and enters only the records unchanged since then (storage/structure/superseded-mark/skip). Also: a failed open loads the catalog, which settles the commit log, before its mapping begins, so its walk writes nothing. Defensive, not the fix for either pinned seed (both pass with it reverted): the commit log's read of a status page it may settle judges the page without refusing or recording it, and a stale copy it does not settle refuses the page when dropped, so a settle in the call that reads the page logs no `Stale` record. Removed the six `SETTLED_MARKS` entries of the damage search (LostCurrent seed 3 write 637, Recycled seed 35 write 641); `a_pinned_known_failure_still_shows` now reruns the `UNREAD_MARKS` verify entry. Defect class: class-not-atomic, the mark set before the record was confirmed current. Seed 35 (Recycled, write 641) passes with the epoch and E11 reverted; only seed 3 was traced. The window between the mapper's currency check and its marks, open to another thread's free or chain start, stays: mapped-record-superseded-before-mark.md.
