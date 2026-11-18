---
created-at: 2026-11-17
status: open
kind: bug
priority: 120
summary: "a migration that rewrites a relation inserts its rows again with fresh large values and frees the old heaps without counting their versions off the old heads, so those values are never freed"
tags:
  - engine
  - class-none
---

# Rewrite leaks large values

## Report

## Report

Found by the retained-guard review (2026-11-17), reproduced on e2a8fb29: after the clean `migration` case of crates/adtb/src/catalog/commit_point_tests.rs, verify finds one `Overcounted { count: 1, versions: 0 }` head per row. `Catalog::build` (crates/adtb/src/catalog/migration.rs) creates new heaps and inserts every row again; `Heap::body` gets no reuse, so the new versions point at new heads. The old heaps go to `gone` and `free_leaked` frees their pages, but nothing counts the old versions off their heads in the shared `LargeStore`, and `sweep` skips any head with a count above 0, so the store pages are never freed. Only the test-only `repair` recounts.

Probably the same: an aborted migration (`free_leaked` drops a new heap whose versions were counted) and dropping a relation. Probe both.

Expected: freeing a heap releases the count of every large value its versions name, in the same step that frees its pages, or the rewrite reuses the old heads. A test: a migration that rewrites a relation with large values, then verify finds no overcounted head and the old values' store pages are freed by the next sweep; the same for an aborted migration and a drop.

Defect class: none fits exactly; a whole-heap free that skips the per-version release.
