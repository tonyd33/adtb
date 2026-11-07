---
created-at: 2026-11-07
status: open
kind: bug
priority: 125
summary: "an open after a close with a durable Database mark on the marks page applies its migration and still holds the mark, so the mark is lost before the open checks it"
tags:
  - class-damage-unreported
---

# A migration applies under a durable whole-database mark

## Report

## 2026-11-07

Found by the damage class search, round 5. Shortest case: LostCurrent Bitmap, seed 3 at write 653, operation 6. Operation 4's open marks the database and does not apply its migration. Before operation 6's close the marks page holds `Database`. The reopen applies the migration, and `Database` is still marked after.

The open skips migration when the mark is loaded, and `quarantine` waits until the mark is durable, so the mark is either lost across the close or not loaded before the open checks it. Cause not found.

Also: LostCurrent Engine (seed 3 at 605), LostCurrent Large (seed 3 at 617), Recycled Bitmap (seed 51 at 647), and ten deep pairs of seeds 3 and 6.

storage.md §8: an open under a whole-database mark applies no migration.

Defect class: Damage reported as nothing.
