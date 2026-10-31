---
created-at: 2026-10-30
status: resolved
kind: bug
priority: 426
summary: "a statement that suspends after a command in one text reruns the command, so a command takes effect twice"
closed: 2026-10-30
---

# Command rerun after suspend

## Report
`set setting X` or `vacuum t` followed in one text by a write that waits on another transaction: the write suspends, the call runs the text again from the first item, and the command takes effect a second time. `resumable` in `api/mod.rs` and `writing` in `api/server/call.rs`. A serialization retry of the same text may have the same result.

Ruling, from quality-fixes33: a command takes effect once. Reads with commands already follow it: only the first command suspends, a later one polls with its stops. This ticket extends the rule to a statement after a command, in reads and writes.

## 2026-10-30

Vacuum, verify and reindex beside a write are refused already; `set setting`, `retire bundle`, `supersede bundle` and `explain` are what can precede one. Ruling ops/command-retry/surface: a text in which a command ran is not rerun on a serialization failure. wire.md 6.4 states it. Includes a commit failure, embedded and server.

Landed. A statement after a command polls with its stops and holds its worker; a text in which a command ran is not rerun. Branch check exit 0, 3329 tests.

## 2026-10-30: resolved
