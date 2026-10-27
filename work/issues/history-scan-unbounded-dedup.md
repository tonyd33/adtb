---
created-at: 2026-10-27
status: open
kind: bug
priority: 895
summary: "a historical scan clones every visible deleted current row into a deduplication map outside the query memory grant, even for count"
tags:
  - quality
  - engine
---

# A historical count retains full deleted rows outside its memory grant

## Report

At `c317ef9a`, `Store::scan_past` (`crates/adtb/src/exec/store.rs:247-276`) keeps `deleted: BTreeMap<(u64, u64), Vec<Vec<Value>>>` for the entire scan. Every visible current version with nonzero `xmax` clones its full row into this map at lines 260-263. The map has no arena, grant charge, spill path or size limit, and is populated even when no copy exists in history. It survives every emitted batch until the scan ends.

Concrete history: enable a sufficiently long retention window, insert many large rows, establish a marker, then delete those rows in a later transaction without vacuuming. An `at commit` count of the earlier state scans the retained current versions and holds another full copy of all their values until its scan finishes. Increasing the row count grows this allocation independently of `OpenOptions::query_memory_bytes`, even though a count has constant-sized output. A large scan can exhaust process memory instead of spilling or returning `StatementLimit`.

When many rows share an inserting/deleting transaction pair, the history half also searches the same vector with `same.contains(&v.row)` for each copy (line 264). That makes deduplication quadratic in that group size.

Fix: bound and charge the cross-walk deduplication state to the query grant, with a spill or limit path, and avoid linear full-row searches per copied row. Add a regression that reads many retained deleted rows with a small grant and constant-sized output; include versions present in both heaps. `docs/design/execution.md:691-698` and the current budgeted-allocation rulings require a call to account for the values it holds.

This is separate from [exec-scan-follow-ups.md](exec-scan-follow-ups.md)'s one batch being filled outside the grant: the new historical path retains whole-scan state after those batches have been consumed. Confirmed by the allocation and lifetime in the code; no allocator-instrumented reproduction was run.
