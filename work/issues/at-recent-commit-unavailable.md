---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 799
summary: "at commit at the position of a commit just made is refused HistoryUnavailable, even with a retention window set and after a reopen, and sys::commit_times is empty"
closed: 2026-10-27
tags:
  - engine
---

# Bug: a read of a recent commit is refused

## Report

Found while writing the manual. After `set setting RetentionWindow = { length: 1h }` and a reopen, `at commit P` at the position of a commit just made fails with `Rejected HistoryUnavailable { oldest: ... }`, and `sys::commit_times` is empty. `derived.md` section 6 says any committed state within the window can be read. Reproduce through the shell as the manual would show it, and through `crates/adtb-sim/tests/past.rs`.

## 2026-10-27: resolved

Resolved in `2d2c1211`. Publish retained commit positions and quiescent safe
snapshots before acknowledgement, restore retention on reopen, and expose
runtime rows through `sys::commit_times` and `sys::safe_snapshots`.
Integrated `check` passed: 2310 workspace tests, 8 skipped, 27 campaign tests,
clippy, formatting, dependency policy and source-size checks.
