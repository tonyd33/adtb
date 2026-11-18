---
created-at: 2026-11-17
status: open
kind: feature
priority: 20
tags: [tooling, test]
summary: "remove the machine lock: a kernel memory limit per test run, timeouts as hang guards only, wall-time tests wait on events"
---

# Feature: Machine lock removal

## Request

`check` and `suite` take one lock for the whole machine, so every agent's run waits for every other. Measured 2026-11-17: a check waited 435 s for another before 841 s of work, and the lead's full check blocked a track's check for its whole length. The lock exists because two concurrent runs once exhausted memory (check-machine-lock): each test process may hold 1.5 GiB and a run uses 16. Tests also depend on the machine being idle: the nextest kill at 60 s fails slow sweeps under load, and some tests order steps by sleeps or fail on a time bound.

## Proposal

Maintainer, 2026-11-17: remove the lock altogether. Replace it with a kernel memory limit per run (a cgroup scope), timeouts that only catch hangs, and tests that wait on events instead of time. Then delete what exists only for the lock.
