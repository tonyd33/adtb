---
created-at: 2026-10-30
status: in-progress
branch: command-rerun
kind: bug
priority: 426
summary: "a statement that suspends after a command in one text reruns the command, so a command takes effect twice"
---

# Command rerun after suspend

## Report
`set setting X` or `vacuum t` followed in one text by a write that waits on another transaction: the write suspends, the call runs the text again from the first item, and the command takes effect a second time. `resumable` in `api/mod.rs` and `writing` in `api/server/call.rs`. A serialization retry of the same text may have the same result.

Ruling, from quality-fixes33: a command takes effect once. Reads with commands already follow it: only the first command suspends, a later one polls with its stops. This ticket extends the rule to a statement after a command, in reads and writes.
