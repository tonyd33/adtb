---
created-at: 2026-11-07
status: open
kind: bug
priority: 136
summary: "a lost write of a commit log status page that the open settles marks the whole database and refuses every read, where storage.md says a settled status page marks nothing"
tags:
  - class-failure-flattened
---

# A settled status page marks the database

## Report

## 2026-11-07

Found by the damage class search, round 2 (gate `SETTLED_MARKS`). LostCurrent CommitLog, seed 3 write 637: Insert users, Insert docs, Reopen, Update users, Migrate, Checkpoint, Migrate, Crash, Checkpoint. Status page 13 is recorded Settled and reads fresh at the end, yet the database holds a `Database` mark and every read is refused "the database". Recycled CommitLog seed 35 write 641 shows the same in the deep tier.

storage.md §8: a status page the commit log reads stale, settles and writes back marks nothing.

Direction: find what sets the mark after the settle, whether the settle's own detection or a later check that judges the page by its pre-settle evidence. The default suite reruns seed 3 write 637 (`a_pinned_known_failure_still_shows`); the fix repoints it.

Defect class: Damage or failure gives a panic, hang or flattened error.
