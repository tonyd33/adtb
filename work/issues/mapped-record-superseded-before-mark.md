---
created-at: 2026-11-08
status: open
kind: bug
priority: 140
tags:
  - class-not-atomic
  - engine
summary: a damage record freed or restarted by another thread after the mapper confirms it current and before its marks are set leaves a durable mark with no damage behind it
---

# A mark outlives the damage another thread clears while the mapper marks

## Report

`Pager::map_records` (crates/adtb/src/storage/damage_log.rs:215) keeps the records still current since the mapping began, then sets their marks (damage_log.rs:225) without the meta lock, since a mark writes the marks page and the log. Between the two, another thread can end the record: a vacuum freeing the damaged page, or an allocation or overwrite starting its chain again. The mapper then sets the marks of a record that no longer exists; `Evidence::mapped` skips the entry, but the mark is durable and holds until `release`, refusing reads of an object with no recorded damage.

Scenario: the end-of-call mapper (crates/adtb/src/write/quarantine.rs:110) maps a heap page's `Checksum` record to `Heap(h)`; after the `current` check, a vacuum on another thread frees the page (its record dropped by the `Freed` meet); the mapper marks `Heap(h)`. Every later read of the heap is refused until a release, though the damage list is empty.

Predates the epoch check of storage owners step 7, which closed the same window for a record superseded by the mapper's own walk.
