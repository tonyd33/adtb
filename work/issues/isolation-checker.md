---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 630
summary: "no checker finds G0, G1 or G2 cycles in a run's history"
closed: 2026-10-25
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

## 2026-10-25: resolved

Landed as `feat(isolation): add the list-append isolation checker and anomaly scenarios`, crate `adtb-isolation`. The workload in `docs/design/runtime.md` §6.4 now appends by inserting at position `count` with `unique (k, pos)`, so two concurrent appends to one key conflict. The engine side (`Database` for the engine, the scenarios and a random-workload driver) is `ssi.md`'s and `isolation-driver.md`'s.
