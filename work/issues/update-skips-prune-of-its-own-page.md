---
created-at: 2026-11-09
status: open
kind: bug
priority: 330
summary: "an update that changes an indexed field and meets its own page full as the insert hint refuses that page without pruning it, so dead versions there stay until vacuum and the version takes another page"
tags:
  - quality
  - engine
  - class-copied-rules
---

# Bug: An update changing an indexed field does not prune its own full page

## Report

mvcc.md §10.2 states that a writer finding a page without room for a new version prunes it before looking further. `Heap::put_successor` (crates/adtb/src/heap/handle/mod.rs:1103) prunes the old version's page `o` only in its heap-only loop (`while keys_kept && tries < 2`, :1126). Its second loop prunes a full candidate only when `n != o` (:1181). That guard is right after the heap-only loop has pruned `o`, but when the update changes an indexed field (`keys_kept` false) the heap-only loop never runs, so `o` is never pruned.

Path: `candidate` returns the insert hint without a directory check (:728), and the hint is `o` whenever the last insert went there. `try_put` answers `Full`, `n == o` skips `prune_full`, and `refused(o)` forgets the hint and writes `o`'s full category to the directory, so later inserts skip `o` as well. The new version goes to another page, often a new one, while `o` keeps dead versions until vacuum's next pass over the relation (for catalog heaps, none: catalog-heaps-never-vacuumed). This is the shape structure-version-owner step 10 fixed for retag.

No test drives a key-changing update onto a full hint page: the heap tests call `update` with `indexed = &[0]` on rows that keep field 0, and write/prune_on_access_tests.rs covers only retag.

Reproduction, reasoned and not run: as in `a_retag_into_a_heap_whose_only_page_is_full_of_dead_versions_reuses_that_page`, fill exactly one page of a relation with a unique field other than its key, so that page is the hint; delete every row but one and commit; then update the remaining row, changing the unique field. Expected: the page is pruned and takes the new version, and `allocated` is unchanged. Actual: a new page is allocated.

Fix direction: skip the prune of `o` in the second loop only when the heap-only loop already pruned it, or prune `o` once before the second loop when `keys_kept` is false; add the case above beside the retag case.
