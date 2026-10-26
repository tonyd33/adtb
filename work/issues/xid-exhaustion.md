---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 690
summary: spending every 64-bit xid panics instead of refusing new transactions
closed: 2026-10-25
depends-on:
  - commit-log-snapshots
tags:
  - engine
---

# Feature: Refuse transactions when xids run out

## Report

`Transactions::begin` panics once every 64-bit xid has been spent. At a
million transactions a second that takes more than 500,000 years, but
the code should still not panic. Expected: `begin` returns an error,
and the database stays readable.

## 2026-10-25: resolved

Landed as `feat(mvcc): quarantine the database on a damaged commit log page and refuse transactions when xids run out`.
