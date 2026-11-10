---
created-at: 2026-11-07
status: resolved
kind: bug
priority: 141
summary: "a mapping walk that meets a lasting unreadable heap page marks the whole database, where storage.md says a walk stopped by an I/O failure is incomplete and marks nothing"
closed: 2026-11-09
tags:
  - class-failure-flattened
---

# A mapping walk stopped by an unreadable page marks the database

## Report

## 2026-11-07

Found by the damage class search, round 2 (gate `UNREAD_MARKS`), deep tier only. UnreadableLasting Heap (users), seed 5 write 620: Insert users, Insert docs, Delete all users, Damage, Checkpoint, Vacuum users, Insert users, Reopen. A `Database` mark is set beside the deliberate scraps mark and reads are refused "the database".

storage.md §8: a mapping walk stopped by an I/O failure is incomplete and marks nothing; the page is refused unmarked.

Direction: find where the walk's failure becomes a whole-database mark. Related: incomplete-mapping-never-retried.

Defect class: Damage or failure gives a panic, hang or flattened error.

## 2026-11-09: fixed

structure-damage-policy step 8. Two causes. The search rows were the search's judge: the pid was freed and reused as the LSN map root, whose unreadable page holds the database for that open (storage.md §10.3); the judge now classes the page by its last incarnation. The engine instance was real at a failed open: `Unopened::holder` read a refused read of the page, an I/O failure included, as holding it and marked the database; the failed open now maps through `Holders::walk`, where an I/O stop leaves the mapping incomplete. Pinned by `an_open_whose_walk_of_the_roots_heap_cannot_read_its_damaged_page_marks_nothing`.

## 2026-11-09: resolved
