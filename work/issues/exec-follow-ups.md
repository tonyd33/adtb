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
