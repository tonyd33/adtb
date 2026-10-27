---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 796
summary: "exec::batches is pub, called only by one sim test, and panics on a mistyped value"
closed: 2026-10-26
tags:
  - quality
---

# Feature: exec::batches returns an internal error or leaves the crate's API

## Report

`exec::batches` (crates/adtb/src/exec/mod.rs:210) panics when the value is not a set or list and, since the accessors became fallible, `expect`s "a row of a batch of fields has fields" (mod.rs:224); `datum` below it panics on a value of another type. Its only caller is crates/adtb-sim/tests/exec.rs:499. Every other accessor path in `exec` now returns `Error::Internal` on a mistyped value.

Fix: return `Result<Vec<Batch>, Error>` with `Error::Internal` for each mistyped value, or, if no engine path will call it, move it into the sim test's support code.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as xtksymmw.
