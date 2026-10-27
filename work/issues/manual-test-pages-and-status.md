---
created-at: 2026-10-27
status: open
kind: bug
priority: 876
summary: "the manual test lists its pages by hand, ignores each session's exit status and writes all input before reading output"
tags: [quality, test]
---

# A new manual page runs no example, and a session that exits 1 passes

## Report

`crates/adtb-shell/tests/manual.rs`:

- `:197-300` has one test per hard-coded page, so a page added under `docs/manual/` runs none of its examples.
- `:156` discards the child's exit status, so a session can print the expected output while the shell exits 1.
- `:146-155` writes all of stdin before reading any output, which deadlocks once a session's output passes the pipe buffer.

Fix: walk `docs/manual` for `.md` files (or keep the named tests and add one that checks every page is listed), assert each session's exit status, and write stdin from a thread.
