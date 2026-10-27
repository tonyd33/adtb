---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 799
summary: "host functions have no bodies in index key evaluation, migration validation or index builds, and a staged job can reach one before its redefine fn"
closed: 2026-10-27
tags:
  - engine
---

# Bug: host functions are missing outside calls

## Report

`OpenOptions.host_fns` binds host function bodies for queries, written values and constraints. `catalog/keying.rs` builds `exec::Scalar` without them, so an index whose key calls a host function cannot evaluate it, and migration validation and index builds have no bodies either; a call that reaches a missing body fails with `Fatal MissingFunction`. A staged job that reaches a host function before its `redefine fn` is not refused. A `Volatile` host function is not held to one evaluation per call.

## 2026-10-27: done

`exec::Hosts` holds each body with its supplied version, and `HostCall` carries the declared version. A call to a body bound at another version fails with `exec::Error::OtherVersion`. Open binds the supplied bodies to the catalog (`Catalog::set_hosts`) before migrations run, so index keys, data maps and constraint validation call them. A staged job that reaches a function before its `redefine fn` fails open with `HostFnMismatch`, and one that reaches an unsupplied function fails with `HostFnMissing`. Write-path index keys get the bodies through `write::Database::with_hosts`. The normaliser treats a call of a non-deterministic host function as a draw, so it is never copied. Tests: five new ones in `host_fns`.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the q799 track.
