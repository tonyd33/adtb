---
created-at: 2026-10-25
status: open
kind: feature
priority: 627
summary: "Transactions takes &mut self, so every commit log status lookup goes through one lock"
tags:
  - engine
---

# Feature: Interior locking in Transactions

## Report

`adtb::mvcc::Transactions` is `&mut self`, so callers wrap it in a lock,
and every commit log status lookup goes through it. Give it interior
locking: lookups on resolved xids should take no lock that a commit
holds.
