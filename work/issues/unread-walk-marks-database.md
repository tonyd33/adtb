---
created-at: 2026-11-07
status: open
kind: bug
priority: 141
summary: "a mapping walk that meets a lasting unreadable heap page marks the whole database, where storage.md says a walk stopped by an I/O failure is incomplete and marks nothing"
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
