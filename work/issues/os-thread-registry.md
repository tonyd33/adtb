---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 335
summary: "OsRuntime registers every thread that asks for its id and never forgets one, under one mutex taken on every pager I/O"
closed: 2026-10-26
tags:
  - quality
  - sim
---

# OsRuntime registers every thread that asks for its id and never forgets one

## Report

`OsRuntime::current` (crates/adtb-sys/src/os/mod.rs:203) inserts the calling thread into the runtime's `Registry` (mod.rs:162) with its `Thread` handle the first time it asks, and nothing removes the entry: only `join` (mod.rs:229) removes one, and only for a spawned thread. A host that serves each client on a thread of its own grows the map by one entry per client thread for the life of the runtime. A spawned thread that is never joined also stays, with its `JoinHandle`.

`Pager::io` (crates/adtb/src/storage/pager.rs:434) calls `rt.current()` on every I/O, so every page read and write also takes the registry's one mutex, shared by every thread of the process.

Fix: keep the `Thread` handle in the thread-local beside the id and register a non-spawned thread only when another thread needs to unpark it, or drop the entry when the thread exits (a thread-local guard's `Drop`); answer `current` from the thread-local alone without the lock.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as mpwvomok.
