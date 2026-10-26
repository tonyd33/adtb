---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 620
summary: "no commit path: commits are not made durable, grouped or given commit positions"
branch: pager
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
