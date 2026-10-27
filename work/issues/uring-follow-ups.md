---
created-at: 2026-10-27
status: open
kind: feature
priority: 311
summary: "Database::open always uses the pool backend, faults cannot be injected under io_uring, and the missing-operation refusal is untested"
tags:
  - engine
---

# Feature: io_uring follow-ups

## Report

- `Database::open` (`crates/adtb/src/api/mod.rs`) always builds `Backend::Pool`. It should try `Backend::Uring` through `OsRuntime::try_new` and fall back to the pool on `Unavailable`, as `docs/design/runtime.md` section 2 states.
- `OsRuntime::with_faults` panics on `Backend::Uring`: the fault injector sits on the pool's synchronous `perform`. Fault injection beneath the ring is missing.
- `Unavailable::Missing`, returned when the kernel lacks an operation the backend needs, has no test.

## 2026-10-27: done

`Database::open` tries `Backend::Uring` through `OsRuntime::try_new` and falls back to `Backend::Pool` on `Unavailable` (`os_runtime` in `crates/adtb/src/api/mod.rs`); stated in `docs/design/server.md` §4.1.

Still open: fault injection under io_uring, and a test of `Unavailable::Missing`.

## 2026-10-27: open falls back

`Database::open` now tries io_uring and falls back to the pool. Fault injection under io_uring and the `Unavailable::Missing` test stay open.
