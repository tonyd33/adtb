---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 400
summary: "the engine has no pager, page format, write-ahead log or recovery"
closed: 2026-10-25
depends-on:
  - page-buffer-contract
---

# Feature: Pager and log

## Report

Milestone 4 of `docs/plan.md` has not started. The engine has no file
header, page format, checksums, buffer pool, redo log, checkpoints or
recovery. `docs/design/storage.md` states the contracts; the simulator
(`adtb-sim`) and the runtime interfaces (`adtb::runtime`) exist to test
against. Exit: random page-write workloads under simulated crashes and
every injected fault lose no acknowledged write and report every
detectable corruption; crash replay passes for every flush boundary.

## 2026-10-25: split

Milestone 4 is tracked by `milestone-4-pager-log.md`. Three parts have
their own tickets so that milestones 5 and 6 can start before the pager
is finished: `page-buffer-contract.md` (the interface heaps, trees and
the commit log build against), `vmcache.md` and
`crash-replay-harness.md`. This ticket keeps the file header, page
format, checksums, the log, checkpoints and recovery.

## 2026-10-25: resolved

Exit criteria met in `feat(storage): page header, allocation and page access interface` and the commits after it: random page-write workloads under crashes and every injected fault lose no acknowledged write and report every detectable corruption, and crash replay passes at every flush boundary. Encryption is milestone 12. Group commit with a leader is `commit-path.md`. Remaining pager work is in `pager-allocate-in-change.md`, `pager-evicted-lsn-bound.md`, `pager-self-deadlock.md`, `quarantine-marks-persisted.md` and `recovery-streams-segments.md`.
