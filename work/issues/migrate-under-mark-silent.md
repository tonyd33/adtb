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
