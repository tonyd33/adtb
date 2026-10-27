---
created-at: 2026-10-27
status: open
kind: bug
priority: 799
summary: "host functions have no bodies in index key evaluation, migration validation or index builds, and a staged job can reach one before its redefine fn"
tags:
  - engine
---

# Bug: host functions are missing outside calls

## Report

`OpenOptions.host_fns` binds host function bodies for queries, written values and constraints. `catalog/keying.rs` builds `exec::Scalar` without them, so an index whose key calls a host function cannot evaluate it, and migration validation and index builds have no bodies either; a call that reaches a missing body fails with `Fatal MissingFunction`. A staged job that reaches a host function before its `redefine fn` is not refused. A `Volatile` host function is not held to one evaluation per call.
