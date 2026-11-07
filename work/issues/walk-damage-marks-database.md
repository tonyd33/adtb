---
created-at: 2026-11-07
status: open
kind: bug
priority: 138
summary: "one stale user-relation page met by another record's mapping walk marks the whole database beside its relation, so every read is refused"
tags:
  - class-failure-flattened
---

# A lost relation page marks the database beside its relation

## Report

## 2026-11-07

Found by the damage class search, round 3 (gate `BESIDE`). One stale user-relation page marks its own relation and also the database; every read is refused "the database". Cells: LostCurrent and Recycled at Directory, Index and VisibilityMap.

- Shortest: seed 5, LostCurrent Directory (docs) write 613 and Index (docs) write 616. Sequence: Insert users, Insert docs, Delete all users, Damage, Checkpoint, Vacuum users, Insert users, Reopen.
- Others: LostCurrent VM (docs) seed 16 write 685; Recycled Directory and Index (docs) seed 26 writes 613 and 616; Recycled VM (users) seed 13 write 630; the pair seed 1, lost 629 then 632.

Every case seen has a deliberately damaged page whose record's mapping walk likely reads the stale page first. storage.md §8 maps damage a mapping walk finds on a page that had no record when the mapping began to the database. Ruling: that sends to the database only a page whose owner cannot be read; a page whose header names its relation maps to that relation, whichever walk finds it. If §8's text allows the wider reading, the spec change goes through the design space with this ruling.

Defect class: Damage or failure gives a panic, hang or flattened error.
