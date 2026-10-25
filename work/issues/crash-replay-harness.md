---
created-at: 2026-10-25
status: open
kind: feature
priority: 403
summary: no harness replays a run to every flush boundary and checks recovery
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
