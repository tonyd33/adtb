---
created-at: 2026-11-07
status: open
kind: bug
priority: 140
summary: "an unreadable LSN map page refuses every relation's reads as the database, where storage.md says such a page is refused unmarked"
tags:
  - class-failure-flattened
---

# An unreadable LSN map page refuses every read as the database

## Report

## 2026-11-07

Found by the damage class search (damage_search in crates/adtb-sim/tests/storage). With one LSN map page unreadable, every read of every relation is refused as "the database". Shortest case: seed 4 write 683, sequence Insert users, Insert docs, Vacuum users, Migrate.

Not yet confirmed: whether a `Database` mark is set, and whether the page was unreadable at the open or only later. storage.md §6 says a map page the device cannot read is refused unmarked, as any page is, and a read of the map that meets it fails. §8 says a map page the open cannot read quarantines the whole database for that open and writes nothing, no mark included. So a refusal of every read as the database for that open, with no mark, is the contract when the open meets the page; a durable mark, or a whole-database refusal from a page first met after the open, is the bug.

Direction: read the mark and the point the page is first met at seed 4 write 683. If the engine conforms, the fix is in the search's judge and this ticket closes as wontfix with that reason.

Defect class: Damage or failure gives a panic, hang or flattened error.
