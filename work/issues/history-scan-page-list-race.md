---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 799
summary: a historical scan misses a retained row when vacuum moves it to a history page allocated after the scan captured its page list
closed: 2026-10-27
tags:
  - quality
  - engine
---

# A historical scan misses a row moved onto a newly allocated history page

## Report

At review head `c317ef9a`, `Store::scan_past` in `crates/adtb/src/exec/store.rs:247-275` calls `past.heap.pages` at line 251 before it reads any current page. The historical reader holds a marker, which permits vacuum to move retained versions to history. If vacuum moves an unread current version onto a history page allocated after line 251 and prunes its original, neither page walk returns it.

A deterministic low-level simulator reproduction ran against this head:

1. Create a current heap with 1,000 `I64` rows, spanning several pages, and an empty history heap.
2. Commit the inserts and take the snapshot that sees them. Delete the final row in a later committed transaction.
3. Bind a `Store` with that earlier snapshot and the current/history `StoredHeap` pair, as an `AtMarker` store does.
4. Run `store.scan(heap_id, 1, each)`. In the first batch callback, move the final row with `past.copy_version(&pager, &heap, &version)`, then prune only that original slot. This deterministically places the move after the history page enumeration but before the scan reaches the final current page.
5. Count the scan's batches. The final row's history copy remains present and visible.

Actual:

```
historical scan: expected 1000 rows, got 999
```

Expected: 1,000 rows, each once. This violates `docs/design/mvcc.md:830-833`, which promises the marker's rows however vacuum moves versions during the read.

Fix the handoff between the two walks: enumerate the history pages after the current walk has finished, and retain deduplication of versions observed in both locations. Add a deterministic regression through `Store::scan` or an API `at` query, with an initially empty history heap. The concurrent test at `crates/adtb-sim/tests/history.rs:1028-1089` exercises `history::rows`, which enumerates each heap when it reaches it, not `Store::scan_past`. `crates/adtb-sim/tests/past.rs:363-376` vacuums between reads, not during one.

The standalone reproduction source is `/tmp/opencode/quality27-history-scan.rs` in this review session; the steps above do not depend on retaining it. Existing past/history/write-state tests pass: 87 passed, one skipped.

## 2026-10-27: resolved

`3a6d6639` enumerates history pages after the current scan. A deterministic 1000-row regression returned 999 rows before the fix and passes afterward, including duplicate suppression and early stop.

Independent review found no blockers. Combined main check exited 0:
2329 root tests passed (8 skipped), 27 campaign tests passed (1 skipped),
root/campaign clippy and fmt, dependency policy and size checks passed.
House checks passed. Evidence: target/integrated-maintenance-check.log
and work/briefs/review-batch-final.md.

## 2026-10-27: resolved
