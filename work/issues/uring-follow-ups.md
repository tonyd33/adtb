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
