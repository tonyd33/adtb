---
created-at: 2026-10-27
status: open
kind: bug
priority: 895
summary: "a vacuum whose history heap write fails on a page error is reported as Internal with Debug text, not as a storage fault"
tags:
  - quality
  - engine
---

# A history heap write failure in vacuum is reported as Internal with Debug text

## Report

`crates/adtb/src/write/mod.rs:707` maps `vacuum::Error::Heap(h)` to `Error::Internal(format!("vacuum wrote history: {h:?}"))`. `heap::Error` (`crates/adtb/src/heap/handle.rs:28-36`) is `Page(PageError)` or `TooLarge`, so an I/O failure or a full device while vacuum copies a version into a history heap reaches the API as an internal error with flattened text instead of a storage fault.

Fix: map `heap::Error::Page(p)` to `Error::Page(p)`, as the arm above does for `vacuum::Error::Page`; keep `TooLarge` as an invariant breach whose text names the invariant.
