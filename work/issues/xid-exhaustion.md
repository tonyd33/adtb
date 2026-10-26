---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 690
summary: spending every 64-bit xid panics instead of refusing new transactions
branch: adtb-txn3
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
