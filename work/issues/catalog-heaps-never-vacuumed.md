---
created-at: 2026-11-09
status: open
kind: bug
priority: 260
summary: "the roots heap and other catalog heaps are never vacuumed, so each dead version that is not heap-only leaves a dead slot no pass frees, growing without bound as kept rows churn"
tags:
  - engine
  - class-unbounded-work
---

# Bug: Catalog heaps collect dead slots no pass frees

## Observed

Vacuum runs per user relation (crates/adtb/src/write/vacuums.rs, keyed by `schema::Rel`); nothing in vacuum/ or write/vacuums.rs touches the roots heap or the other catalog heaps. Since structure-version-owner step 10, a catalog insert prunes a full page on access, which frees dead versions but leaves each dead version that is not heap-only as a dead slot, and only vacuum's index pass frees a dead slot. The roots heap is found by scan, so its dead slots are never freed. Before step 10 whole dead versions collected there instead; the growth is now slower but still unbounded under kept-row churn.

## Expected

Every heap's dead slots are freed by some pass with a stated bound: either catalog heaps are vacuumed (an index pass where they have indexes, a slot release where they are found by scan), or a scanned heap's prune frees a dead slot directly.

## Reproduction

Churn kept rows (insert and delete) in a loop, then count dead slots in the roots heap: they grow with the number of iterations. Found by the step 10 review of structure-version-owner.
