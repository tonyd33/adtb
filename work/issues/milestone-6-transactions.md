---
created-at: 2026-10-25
status: open
kind: feature
priority: 600
summary: "milestone 6 (transactions) has not exited: no commit log, snapshots, SSI, commit path or isolation checker"
depends-on:
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
