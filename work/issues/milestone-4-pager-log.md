---
created-at: 2026-10-25
status: open
kind: feature
priority: 400
summary: "milestone 4 (pager and log) has not exited: no page contract, buffer pool, log, recovery or crash replay"
depends-on:
  - pager-allocate-in-change
  - quarantine-marks-persisted
  - recovery-streams-segments
  - page-buffer-contract
  - pager-and-log
  - vmcache
  - crash-replay-harness
tags:
  - milestone
---

# Feature: Milestone 4, pager and log

## Report

Milestone 4 of `docs/plan.md`, on the critical path. Exit: random
page-write workloads under simulated crashes and every injected fault
lose no acknowledged write and report every detectable corruption;
crash replay passes for every flush boundary.

Open on 2026-10-25: `page-buffer-contract.md` (first: milestones 5
and 6 build against it), `pager-and-log.md`, `vmcache.md`,
`crash-replay-harness.md`.
