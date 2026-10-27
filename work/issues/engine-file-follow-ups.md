---
created-at: 2026-10-26
status: open
kind: bug
priority: 790
summary: "the engine reads every file scan of a statement up front, refuses a file path that depends on a row, and reads files with std::fs instead of the runtime"
tags:
  - engine
---

# Bug: Engine file scan follow ups

## Report

Left by the engine file scans track:
- Every file scan of a statement is read when the statement starts, so a bad record fails the statement even when its scan sits on a branch that is never taken. The reference reads it only when evaluated.
- A file scan whose path depends on a row is refused as `Unsupported`.
- Files are read through `std::fs`, not the runtime I/O interface, so the simulator cannot inject faults into them.
