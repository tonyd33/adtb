---
created-at: 2026-11-09
status: open
kind: bug
priority: 200
summary: "no damage search operation swaps a tree, so the retired-page rows of storage.md §8 are never asked"
tags:
  - testing
  - class-weak-tests
---

# Bug: the damage search never retires a page

## Observed

storage.md §8 states that a page of a tree a swap replaced marks nothing. No operation in the damage search's sequences swaps a tree (a reindex or a rebuild), so `Held::Retired` stays in `unasked()` as `Never`.

## Expected

The search's operations include one that swaps a tree, and the retired rows are asked at the damage tier.

## Reproduction

Read `unasked()` after the damage profile. Found in structure-damage-policy step 2, round 3.
