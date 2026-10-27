---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 599
summary: "a stale tid into a reused slot of another sum's heap, flagged RETAGGED_IN with xmin equal to the predecessor's xmax, still passes the version walk's owner checks"
closed: 2026-10-27
tags:
  - engine
---

# Bug: a version walk can follow a stale tid into another sum's heap

## Report

From the ebug track. `heap::walk` now ends a chain at a version on a page whose owner does not fit: the first version must be in the heap the walk starts from, a MOVED version's successor must be in another heap and flagged RETAGGED_IN, and any other successor stays in its predecessor's heap.

The MOVED rule only asks for another heap. A stale tid that lands in a reused slot of a heap belonging to a different sum, where that version happens to carry RETAGGED_IN and an `xmin` equal to the predecessor's `xmax`, passes every check and the walk returns a version of the wrong relation. It is unlikely, but nothing rules it out by construction.

Fix: give `walk` the sum's heap set, so a retag successor must be in a heap of the same sum, or give the large-value store and other structures their own page kind (which `docs/design/mvcc.md` currently rules out). Add a deterministic test that forges the case.

## 2026-10-27: done

`heap::walk` takes the heaps of the starting heap's relation; a moved version's successor must be in another heap of that set. `StoredHeap` carries the set as `sum`, and `history::probe` takes it beside the current heap. `mvcc.md` §5 states it. `vacuum.rs` forges the case: a stale `next` into a retagged-in version of another sum's heap with the matching `xmin` ends the chain.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qdrops track.
