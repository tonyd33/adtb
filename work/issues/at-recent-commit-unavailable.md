---
created-at: 2026-10-27
status: open
kind: bug
priority: 799
summary: "at commit at the position of a commit just made is refused HistoryUnavailable, even with a retention window set and after a reopen, and sys::commit_times is empty"
tags:
  - engine
---

# Bug: a read of a recent commit is refused

## Report

Found while writing the manual. After `set setting RetentionWindow = { length: 1h }` and a reopen, `at commit P` at the position of a commit just made fails with `Rejected HistoryUnavailable { oldest: ... }`, and `sys::commit_times` is empty. `derived.md` section 6 says any committed state within the window can be read. Reproduce through the shell as the manual would show it, and through `crates/adtb-sim/tests/past.rs`.
