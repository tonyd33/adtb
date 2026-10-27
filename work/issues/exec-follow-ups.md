---
created-at: 2026-10-25
status: open
kind: feature
priority: 722
summary: "heap scans are serial per heap, nothing spills, the interpreter is row at a time, and a plan does not record its result type"
depends-on:
  - executor
tags:
  - engine
---

# Feature: Executor follow-ups

## Report

From the first executor:
- `Heap::scan` reads a whole heap through one callback, so one heap's
  page reads stay serial. Add a scan over a range of directory
  positions, and make one morsel task per range;
- plans carry no memory estimate and nothing spills (`docs/design/
  execution.md` §7.5); past its grant a query fails;
- instructions loop per row over value columns, not Arrow kernels, and
  rows expanded by Unnest, Probe and Index are not re-chunked into
  morsels;
- the scheduler has one shared queue, with no per-worker queues to
  steal from and no credit flow control;
- `Plan` does not record the root's type, which a caller needs to read
  the result.

## 2026-10-26: done

- Spilling: a query's emitted values, hash builds and `Group` sinks
  spill to a per-call spill file (`exec/spill.rs`, `exec/join.rs`):
  emitted sets as sorted runs merged at most 64 at a time, builds as
  16 hash partitions with probing rows deferred and replayed per
  partition chunk, groups through a spilling set of key and element
  pairs. `Top` and `Present` hold only what they return; `Agg` and
  `Fold` fold a morsel at a time. `Options::spill_bytes` bounds the
  file; past it the statement fails with `StatementLimit`. Simulator
  tests force spills of joins, groups, aggregates and sorts.
- `Plan::ty` and `Program::ty` record the root's type.
- `Heap::scan_pages` and `Store::scan_pages` scan a range of the
  heap's pages; a parallel scan runs one task per `range_pages` pages.
- Rows expanded by `Unnest`, `Probe` and `Index` go on in frames of at
  most `morsel_rows` rows.

Still open: Arrow-kernel instructions, per-worker queues with
stealing, credit flow control, a plan's grant estimate, spilling to
pool pages first, encrypted spill files, the open-time spill
directory.

## 2026-10-26: landed, and what stays open

Spilling, the plan's result type, page-range parallel scans and morsel-sized expansion landed with the exec track, merged with the plan track's early stop and range probes. Still open:
- Batch interpretation with Arrow kernels: instructions still loop row by row over `Val` columns, and it needs `Frame` and `Val` redesigned as Arrow arrays.
- API calls do not run on `exec::Scheduler` (the API passes `pool: None`), so no benchmark can show a parallel speedup; `bench_scans_joins_and_groups` in `crates/adtb-sim/tests/exec.rs` measures task overhead only.
- Probers spin-yield on a build slot's lock while the first task builds it (a 4-worker join count takes 360 ms against 20 ms serial): build eagerly or have waiters help.
- No grant estimate from the plan at admission.
- Spill goes to files only, not to pool pages first; spill files are not encrypted; stale spill files are not removed at open after a crash.
- A single oversized group, and group records that hold their rows, do not spill.
- Per-worker queues with stealing, and credit flow control.
