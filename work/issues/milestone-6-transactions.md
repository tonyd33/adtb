---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 600
summary: "milestone 6 (transactions) has not exited: no commit log, snapshots, SSI, commit path or isolation checker"
closed: 2026-10-26
depends-on:
  - pager-safe-snapshot-record
  - txn-interior-locking
  - vacuum-concurrent
  - ssi-integration
  - isolation-driver
  - clog-damaged-page
  - xid-exhaustion
  - milestone-5-heaps-indexes
  - commit-log-snapshots
  - ssi
  - unique-checks
  - commit-path
  - isolation-checker
  - vacuum-threads-xid-not-reserved
tags:
  - milestone
---

# Feature: Milestone 6, transactions

## Report

Milestone 6 of `docs/plan.md`, on the critical path. Exit: no G0, G1
or G2 cycle over a fixed number of seeds per run; the write-skew tests
abort one side. Needs milestone 5; `commit-log-snapshots.md` starts
once `page-buffer-contract.md` is done.

Open on 2026-10-25: `commit-log-snapshots.md`, `ssi.md`,
`unique-checks.md`, `commit-path.md`, `isolation-checker.md`.

## 2026-10-26: gate

The deep-profile vacuum crash (vacuum-threads-xid-not-reserved) gates the exit.

## 2026-10-26: resolved

## 2026-10-26: exited

Every dependency is resolved. `ssi.rs` `write_skew_aborts_one_side` and the isolation checker sweeps pass in the default run, and the deep-profile vacuum crash is fixed.
