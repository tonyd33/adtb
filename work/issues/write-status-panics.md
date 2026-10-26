---
created-at: 2026-10-25
status: open
kind: bug
priority: 695
summary: a commit log read that fails during a write panics the process instead of failing the statement
tags:
  - quality
  - engine
---

# Bug: a failed commit log read during a write panics

## Report

`Stmt::status` (crates/adtb/src/write/apply.rs:38-45) calls `Transactions::status` and `.expect("the commit log of an open database reads")`, typing the result `Result<Status, Infallible>`. `Transactions::status` fails with a `PageError` on a damaged commit log page and on every call once the database is quarantined (mvcc/txn.rs:205). Both are reachable under fault injection, so `writable` (`check_write`) and `entries` (`probe_unique`) panic the process instead of failing the statement with `Error::Page`, which ends the transaction.

Repro: in the simulator, quarantine the database (fail a commit log page read), then run a statement that deletes or replaces a row whose version another transaction deleted, or inserts into a unique index holding an equal key.

Fix: `check_write` and `probe_unique` are generic over the error; pass `PageError` through and map it with `?`. No other change needed.
