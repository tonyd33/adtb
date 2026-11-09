---
created-at: 2026-11-09
status: open
kind: bug
priority: 330
summary: "a stale commit-times page fails the open with Corrupt (Transactions(History(Stale))) instead of opening under the database mark, because neither holders walk reaches the commit-times tree"
tags:
  - class-failure-flattened
---

# Bug: a stale commit-times page fails the open

## Observed

With a retention window set, a lost or recycled write of a commit-times tree page makes the open fail with `Corrupt` (`Transactions(History(Stale))`). Neither holders walk reaches the tree: `holders` in crates/adtb/src/write/quarantine.rs and `Catalog::holders` in crates/adtb/src/catalog/mod.rs. The page is never mapped to a holder, so no mark is set.

## Expected

storage.md §8: damage to the commit-times tree marks the database. A failed open that found no damaged page, or an unreadable one, still fails; this open found a damaged page, so it opens under the database mark.

## Reproduction

The damage search pins it in `KNOWN`: LostCurrent, Engine, Settle, the open fails, seed 4 at 607; Recycled, same cell, seed 10 at 607. Found by the damage search in structure-damage-policy step 2.

## Direction

structure-damage-policy step 3 replaces both holders walks with one `catalog::Structures`; the tree belongs in it.
