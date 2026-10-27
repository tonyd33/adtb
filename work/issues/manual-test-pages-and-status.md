---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 876
summary: "the manual test lists its pages by hand, ignores each session's exit status and writes all input before reading output"
closed: 2026-10-27
tags:
  - quality
  - test
---

# A new manual page runs no example, and a session that exits 1 passes

## Report

`crates/adtb-shell/tests/manual.rs`:

- `:197-300` has one test per hard-coded page, so a page added under `docs/manual/` runs none of its examples.
- `:156` discards the child's exit status, so a session can print the expected output while the shell exits 1.
- `:146-155` writes all of stdin before reading any output, which deadlocks once a session's output passes the pipe buffer.

Fix: walk `docs/manual` for `.md` files (or keep the named tests and add one that checks every page is listed), assert each session's exit status, and write stdin from a thread.

## 2026-10-27: done

`crates/adtb-shell/tests/manual.rs` walks `docs/manual` for `.md` pages and checks them in one test, a thread per page. Each session's exit status is asserted against a `<!-- exit N... -->` line before its `console` block (one status per session, 0 without one); the 73 blocks with a session that exits 1 under piped input carry one. Stdin is written from a scoped thread while output is read. Checked by hand with a scratch page: a 30 000-statement session passes, a wrong status fails with both statuses shown, and a status list of the wrong length fails. Nothing stays open.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qtools track.
