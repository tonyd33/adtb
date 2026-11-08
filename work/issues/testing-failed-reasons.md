---
created-at: 2026-11-08
status: open
kind: bug
priority: 900
summary: "testing::Failed's NotKey doc carries a stale line, testing.md omits NotKey, and check_indexes answers Absent for an object that exists but is no index"
tags:
  - quality
  - engine
  - design
---

# testing controls answer the wrong or an undocumented Failed reason

## Report

- crates/adtb/src/testing/mod.rs:89: the doc of `Failed::NotKey` begins with a stale line, "The database has no relation of the name given.", left from a removed variant.
- docs/design/testing.md does not name `Failed::NotKey` among the reasons a control answers.
- `Testing::check_indexes` (crates/adtb/src/testing/control.rs:152) answers `Failed::Absent` for an object that exists but is not an `Index` (a heap, a history index), while `Absent` means the database has no such structure; `probe_heaps` (control.rs:176) answers `NotKey` for the same case.

Fix direction: delete the stale line, give a non-index object its own reason (or answer as `probe_heaps` does), and state each reason in testing.md §3.
