---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 620
summary: "no commit path: commits are not made durable, grouped or given commit positions"
closed: 2026-10-25
depends-on:
  - commit-log-snapshots
  - pager-and-log
tags:
  - engine
---

# Feature: commit path with group commit

## Report

Implement the commit path: `docs/design/storage.md` §6 and §10.1
(write-ahead and group commit), `docs/spec/transactions.md` §7 at
durability level local and §8 commit positions.

Prerequisites: `commit-log-snapshots.md`, `pager-and-log.md`.

## 2026-10-25: resolved

Landed as `feat(storage): commit through a group-commit leader, log safe snapshots and size threads to the pool`.
