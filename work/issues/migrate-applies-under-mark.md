---
created-at: 2026-11-07
status: open
kind: bug
priority: 125
summary: "an open's migration commits and its post-commit free_leaked meets damage, so Catalog::apply returns Err after its commit is durable; the open fails to a verify-only engine and the next open, under the mark, is charged with the migration"
tags:
  - class-not-atomic
---

# A migration commits and its open then fails on damage

## Report

## 2026-11-07

Found by the damage class search, round 5. Shortest case: LostCurrent Bitmap, seed 3 at write 653, operation 6. Operation 4's open marks the database and does not apply its migration. Before operation 6's close the marks page holds `Database`. The reopen applies the migration, and `Database` is still marked after.

The open skips migration when the mark is loaded, and `quarantine` waits until the mark is durable, so the mark is either lost across the close or not loaded before the open checks it. Cause not found.

Also: LostCurrent Engine (seed 3 at 605), LostCurrent Large (seed 3 at 617), Recycled Bitmap (seed 51 at 647), and ten deep pairs of seeds 3 and 6.

storage.md §8: an open under a whole-database mark applies no migration.

Defect class: Damage reported as nothing.

## 2026-11-09, re-scoped

structure-damage-policy step 5 made every mark durable under a held mark; all 5 rows still fail, and durability was not their cause. In each, a writable open's `Catalog::apply` commits the migration, then `free_leaked` meets a stale page and returns `Corrupt`: LostCurrent Bitmap 3 at 659 (pid 2), LostCurrent Engine 3 at 606 (pid 8), LostCurrent Large 3 at 622 (pid 557), Recycled Bitmap 51 at 651 (pid 2), LostEarlier Large 58 at 623 (pid 558). `mark_failed_open_damage` marks the database and the open returns the verify-only engine, whose schema has no app relations; the next open loads the mark, migrates nothing, and finds the committed migration, so the search charges it. No open applies a migration under a durable mark.

Defect: `Catalog::apply` returns `Err` after its commit is durable, so every caller is told a migration failed that every later open has. Damage the post-commit cleanup meets should be recorded and mapped at the next mapping point, not fail a committed migration. Class: multi-step change not atomic under failure. Fixing it removes the 5 rows. The search reading the stored version instead of inferring it is damage-search-reads-stored-version.

## 2026-11-10

Structure-damage-policy step 10 made the search's judge tell an unread open from a read one, through `verify` of a relation failing with the quarantine. Recycled Bitmap seed 65 at write-out 678 now reaches no finding: the migration commits, a later step meets damage, and the open answers an unread `Ok`, which embedding.md §2.1 step 8 now contradicts (an open under a whole mark applies nothing). The case reaches no later open that would read the applied migration, so the search no longer sees it. A test for this ticket must open again after such an open and read the version.

Scope, from the step 10 review: any failure in open after a migration commits, not only `Catalog::apply`. Damage met later in open after a successful `migrate` also rescues to an unread `Ok` with the migration applied.
