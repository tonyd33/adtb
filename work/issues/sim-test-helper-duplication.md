---
created-at: 2026-10-26
status: open
kind: feature
priority: 771
summary: "commands.rs and files.rs re-add failure, is_invalid, run and other helpers that belong in support/api.rs"
tags:
  - quality
  - sim
  - test
---

# Feature: one copy of each sim API test helper in support/api.rs

## Report

In the range that landed `crates/adtb-sim/tests/support/api.rs`, the new test files re-add helpers it should hold:

- `failure(db, text)` is copied verbatim in `commands.rs:44-63` and `files.rs:77-100`.
- `is_invalid` is in both `api.rs:81` and `commands.rs`.
- Each file has its own `run(db, text)` with a different return type, shadowing the support module's `run(sim, ..)`.
- `names` in `commands.rs:78-82` is `sorted(result(..))`.
- `in_txn_run` and `last` in `files.rs:859-870` are not specific to files.

Fix: move them into `support/api.rs` under one name each.
