---
created-at: 2026-10-29
status: open
kind: bug
priority: 530
summary: The synchronous in-process call path does not pass admission
---

# The synchronous in-process call path does not pass admission

## Report

Spec revision 30 (G18) states every in-process call passes admission (docs/spec/embedding.md
4.1). Only the task path in crates/adtb/src/api/server/task.rs calls `admit`; the synchronous
call path does not. Fix: admit on that path too, with a test that a call over the byte bound is
rejected there.
