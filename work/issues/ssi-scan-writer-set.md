---
created-at: 2026-10-25
status: open
kind: bug
priority: 645
summary: "a heap scan collects every distinct writer of the heap and reads each one's commit status a second time"
tags: [quality, engine]
---

# A heap scan collects every distinct writer of the heap and reads each one's commit status a second time

## Report

`Store::scan` (crates/adtb/src/exec/store.rs:198-222) puts `writer_of` of every version it visits into a `BTreeSet`, whatever the version's age, then calls `AtSnapshot::depends` (store.rs:112-132) once per writer. `depends` asks `counts_committed` and then `status` again, so each writer's commit log entry is read up to twice more after `sees` already read it. A full scan of a heap whose rows were inserted by a million transactions builds a set of a million xids and does a million or two extra commit log reads, for no dependency: every writer below the snapshot's `xmin` finished before the snapshot, so it either counts as committed or aborted, and `depends` returns `Ok` for both. `Store::fetch` (store.rs:230-249) does the same per chain.

Fix: only keep a writer at or above `snapshot.xmin` (and not the reader), or have `Reads::sees` return the writer its answer depended on when that writer is neither counted as committed nor aborted, so the status is read once. Check `Status` once in `depends` instead of through `counts_committed` and `status` separately.
