---
created-at: 2026-10-27
status: open
kind: bug
priority: 897
summary: "the io_uring fallback test builds its own fallback, so nothing tests that Database::open falls back to the pool where io_uring is denied"
tags:
  - quality
  - sim
  - test
---

# The io_uring fallback test builds its own fallback

## Report

`the_pool_is_the_fallback_where_io_uring_is_denied` (`crates/adtb-sys/src/os/tests.rs:126-135`) writes its own `try_new(Uring).or_else(try_new(Pool))` and asserts only `is_ok()`. It never reaches the real fallback, `os_runtime` in `crates/adtb/src/api/mod.rs:378-387`, which also discards the `Unavailable` reason with `unwrap_or_else(|_| ...)`, so nothing shows which backend a host ended up on.

Fix: a test that opens a `Database` on a thread under `deny_io_uring_setup` and asserts it opens and a write round-trips. Consider reporting the chosen backend. The untested `Unavailable::Missing` stays with `uring-follow-ups`.
