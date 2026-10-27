---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 866
summary: "a heap scan reads every batch of its pages into memory outside the grant before the first morsel runs, so spilling does not bound a scan and an early-stopped exists still reads the whole heap"
closed: 2026-10-27
tags:
  - quality
  - engine
---

# A heap scan holds the whole heap in memory, uncharged, and an early stop saves no reads

## Report

`Store::scan_pages` at `crates/adtb/src/exec/store.rs:205-240` pushes every batch of the heap or page range into `out: Vec<Batch>` and returns it, then records every writer it saw with `reads.depends`. `pipeline` (`crates/adtb/src/exec/run.rs:575-578`) and `range_task` (`run.rs:554-559`) consume that vector, and the grant is taken per morsel only afterwards (`run.rs:604`).

- `docs/design/execution.md` (Memory) states a call holds the rows of each morsel while the morsel runs against its grant. A serial pipeline over N pages holds all N pages of rows at once, uncharged.
- `run.stopped()` is checked per morsel, after every page has been read and every writer recorded, so the early stop of `exists` and empty top queries saves chain work but no I/O and no SSI dependencies: the stopped query still conflicts with writers of rows it never needed.
- The spill tests in `crates/adtb-sim/tests/exec.rs` pass because their heaps are small.

Fix: have `scan_pages` yield one batch at a time (an iterator or a per-batch callback), charge and run each batch before reading the next page, stop when the run stops, and record writer dependencies per batch.

## 2026-10-27: done

`Store::scan` and `Store::scan_pages` pass batches to a callback that returns `ControlFlow`, reading one page at a time and recording writer dependencies per batch; `morsel` breaks once the run stops. Tests in `crates/adtb-sim/tests/exec.rs`: a scan passes its first batch before reading the heap's last page, an early-stopped `exists` reads under half the heap's versions, and a heap larger than the grant scans within it.

Open: `Store::pages` still takes the heap-level SIREAD lock up front, so a stopped scan conflicts with every writer of the heap. The batch being filled is held uncharged until the morsel runs.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qexec track; what stays open is in `exec-scan-follow-ups`.
