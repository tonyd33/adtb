---
created-at: 2026-11-07
status: open
kind: bug
priority: 137
summary: "an open given a migration under a whole-database mark applies nothing and returns Ok, so the caller is told the migration succeeded"
tags:
  - class-damage-unreported
---

# A migration under a whole-database mark returns Ok unapplied

## Report

## 2026-11-07

Found by the damage class search, round 2, seed 3 write 608. A lost catalog index page marks the whole database durably. The next open, given the drop migration, runs under the mark, applies no migration, as storage.md §8 says, and returns Ok. The caller is told nothing. The search's model then counted the migration as acknowledged.

Ruling: an open given a migration it does not apply under a whole-database mark refuses with the quarantine, naming the database. Check the spec's open and migration parts for a stated outcome first; if none, the refusal goes in the spec through the spec-revision workflow.

Direction: the search judges a Migrate under a whole-database mark that returns Ok as a violation, gated under this ticket.

Defect class: Damage reported as nothing.

## 2026-11-10

structure-damage-policy step 9 removes 6 of this ticket's search rows (Recycled Directory seed 26 at 617, 618, 633; Recycled Index seed 26 at 620, 621, 635): their whole-database mark came from a met page, which now marks its relation. The defect is not fixed by that. Those seeds now migrate under a relation mark, which the judge does not check (it judges a migration only under the database mark); step 10 decides what a migration under a relation mark owes the caller. 18 rows remain.
