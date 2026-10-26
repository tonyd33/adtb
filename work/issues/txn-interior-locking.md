---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 627
summary: "Transactions takes &mut self, so every commit log status lookup goes through one lock"
closed: 2026-10-25
tags:
  - engine
---

# Feature: Interior locking in Transactions

## Report

`adtb::mvcc::Transactions` is `&mut self`, so callers wrap it in a lock,
and every commit log status lookup goes through it. Give it interior
locking: lookups on resolved xids should take no lock that a commit
holds.

## 2026-10-25: resolved

Landed as `feat(mvcc): lock Transactions internally and catch reads of unflushed commits`. The targeted test `a_reader_of_an_unflushed_commit_is_acknowledged_only_once_it_is_durable` fails on every seed when `read_upto` is ignored.
