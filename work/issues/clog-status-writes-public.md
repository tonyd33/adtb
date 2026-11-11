---
created-at: 2026-11-11
status: open
kind: bug
priority: 500
summary: "CommitLog::commit_at and CommitLog::abort are pub and Transactions::commit_log hands any crate module the guard they need, so a status can be recorded past running and unresolved"
tags:
  - quality
  - engine
  - class-admission-per-path
---

# Commit-log status writes are reachable past Transactions

## Report

`CommitLog::commit_at` (crates/adtb/src/mvcc/clog.rs:573) and `CommitLog::abort` (clog.rs:586) are `pub` and take `&self`; `Transactions::commit_log()` (crates/adtb/src/mvcc/txn.rs:250) returns the read guard to any module of the crate. A status recorded through them bypasses `running` and `unresolved`, which `Begun`'s ending and `resolved_below` depend on (mvcc.md §4.1). `CommitLog::commit` beside them is already testing only. No production caller outside mvcc uses them today; the only outside callers are tests (storage/clog_lost_tests.rs:1185 and :1205).

Direction: make the status writers private to mvcc, or `cfg(any(test, feature = "testing"))` as `commit` is, and give the lost-write tests a testing entry.

Defect class: Authority and admission checked per entry path.
