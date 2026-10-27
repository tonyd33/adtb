---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 691
summary: "a statement that reads a file and fails restores the transaction's local state and drops the file's pin, so a later read of the changed file gets no FileChanged"
closed: 2026-10-26
tags:
  - quality
  - engine
---

# A file read by a statement that fails is not pinned, so a later read of the changed file succeeds

## Report

The first read of a path in a transaction must record its checksums, and every later read must match them or fail with `FileChanged`. `Local::scan` (`crates/adtb/src/write/input.rs:193-199`) records the pin in `local`, but `statements_with` and `trial` (`crates/adtb/src/write/mod.rs:550-553` and `:586`, `*local = before`) restore `local` to its state before the statement whenever the statement fails without ending the transaction. A statement that reads a file and then fails, for example with `BadInput` from a record that does not convert, leaves no pin. In an interactive transaction a later statement can then read the changed file and get its new contents with no `FileChanged`.

Found by reading; no repro yet.

Fix: keep file pins apart from the temp relation state that a failed statement rolls back, and never revert pins on a statement failure. Add a sim test: read a file in a statement that fails, change the file, read it again in the same transaction, expect `FileChanged`.

## 2026-10-26: done

`Local::restore_temps` puts back only the temp relation rows; `statements_with` and `trial` use it, so a file read by a failed statement or by `explain analyze` stays pinned. Tests in `adtb-sim/tests/files.rs`: after a failed scan, and after an `explain analyze` scan, a changed file makes the next read fail with `FileChanged`.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the ebug track.
