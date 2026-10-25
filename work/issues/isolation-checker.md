---
created-at: 2026-10-25
status: open
kind: feature
priority: 630
summary: "no checker finds G0, G1 or G2 cycles in a run's history"
tags:
  - engine
  - test
---

# Feature: isolation checker on the append workload

## Report

Implement the isolation checker of `docs/design/runtime.md` §6.4 over
the append workload, and the write-skew tests. Milestone 6 exits when
no G0, G1 or G2 cycle appears over a fixed number of seeds per run and
the write-skew tests abort one side. The checker reads histories and can
be built and tested on hand-written histories before SSI runs.
