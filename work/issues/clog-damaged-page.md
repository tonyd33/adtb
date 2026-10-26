---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 610
summary: a damaged commit log page leaves its transactions running; nothing defines what it stops
closed: 2026-10-25
depends-on:
  - commit-log-snapshots
tags:
  - engine
  - design
---

# Bug: A damaged commit log page

## Report

A commit log page that fails verification has no heap or index to
quarantine (`docs/design/storage.md` §8). Today `commit` and `abort`
fail with `PageError` and the transaction stays running, and every
visibility test over its ids fails.

My ruling: the commit log is part of every relation, so a damaged
commit log page quarantines the whole database. It opens for `verify`
and repair only, and every transaction is refused with the corruption
fault. State it in mvcc.md §4.2 and storage.md §8, and test it under
the simulator.

## 2026-10-25: resolved

Landed as `feat(mvcc): quarantine the database on a damaged commit log page and refuse transactions when xids run out`.
