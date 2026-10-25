---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 403
summary: no harness replays a run to every flush boundary and checks recovery
closed: 2026-10-25
depends-on:
  - pager-and-log
tags:
  - engine
  - test
---

# Feature: crash replay harness

## Report

Milestone 4 exits when crash replay passes for every flush boundary.
Implement the harness of `docs/design/runtime.md` §6.3 over
`SimStorage`: record a run's flushes, crash at each boundary, recover,
and check that no acknowledged write is lost and every detectable
corruption is reported (`docs/design/storage.md` §3 and §7).

Prerequisite: the log and recovery of `pager-and-log.md`.

## 2026-10-25: resolved

Done with the pager checkpoint, `feat(storage): page header, allocation and page access interface`. The recording API is `Sim::record`, `recording` and `with_files`, `Recording::flushes` and `images`, and `SimRuntime::recorded`. `crates/adtb-sim/tests/pager.rs` crashes at every flush boundary, recovers, and checks that no acknowledged write is lost and every detectable corruption is reported.
