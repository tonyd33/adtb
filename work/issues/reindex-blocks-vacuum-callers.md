---
created-at: 2026-10-30
status: in-progress
branch: quality-fixes33
kind: bug
priority: 655
summary: "vacuum, verify and set_retention wait on the vacuum lock for the whole of a running reindex, holding a worker and ignoring cancel and deadline"
tags:
  - quality
  - engine
---

# Reindex blocks vacuum callers

## Report

A reindex job holds `Database::vacuuming` from `crates/adtb/src/api/reindex.rs:145` until the job ends, every named index included. The other holders take it with a plain lock: `vacuum` at `crates/adtb/src/write/mod.rs:892`, `set_retention` at `crates/adtb/src/write/past.rs:365`, and `check_indexes` at `crates/adtb/src/write/verify.rs:249`.

A `vacuum` or `verify` task therefore sits on a scheduler worker for the rest of the rebuild, and its cancel token and deadline do nothing until the lock comes. `indexes.md` section 7 gives a gated writer no latch and no worker; these callers have neither guarantee. Enough of them fill the workers, which `a_read_runs_while_gated_writes_fill_the_workers` guards only for writes.

Found by reading; not run.

Fix direction: poll a try-lock against the call's stops, as `wait_for_rebuilds` does, or suspend the task on the job as a gated write does.
