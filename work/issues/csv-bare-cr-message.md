---
created-at: 2026-10-26
status: open
kind: bug
priority: 764
summary: a bare carriage return in a CSV file fails the whole file with a message about a quoted cell
tags:
  - quality
  - engine
---

# A bare carriage return in a CSV file is reported as a quoted cell ending early

## Report

A carriage return not followed by a line feed after an unquoted cell (old Mac line endings, or a CR inside an unquoted cell) falls into the quoted-cell branch of `split` (`crates/adtb-file/src/csv.rs:83-86`), and the whole file fails at line 0.

The file `id,name\r1,ann\r`:

Actual: `BadInput` line 0 `byte 7: a quoted cell ends before its separator`.

Expected: either the file reads as two records, or the message names the bare carriage return and its line.

Fix: give the bare CR its own case in `split`, decide whether it separates records, and test it.
