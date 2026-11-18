---
created-at: 2026-11-17
status: resolved
closed: 2026-11-18
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

## 2026-11-18: resolved

Landed in `refactor!: replace the machine lock with a memory limit per run and wait on events in tests`. Each `suite` or `check` run runs through `work/tools/limited` in a systemd user scope of 14G under `adtb.slice`, which is throttled past 60% of memory and capped at 75%, without swap; a process past a limit is killed and the run goes on. The nextest kill is a 300 s hang guard. Test scratch paths are per process. The wall-time tests wait on events, except dir_tests' latch sleeps (latches spin with no event; the sleeps only weaken the test) and the drop test, renamed to what it checks. Three suites started together passed in about 557 s each against about 180 s alone, throttled and none killed. Main's first lockless `check` passed; its scope's peak reached the 14G limit, counting page cache, with nothing killed. Unlimited: a typed `cargo nextest run` or `cargo test`, the other tools, CI. CPU is not bounded.
