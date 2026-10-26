---
created-at: 2026-10-26
status: open
kind: feature
priority: 794
summary: "Pager is now a cloneable handle over Arc<Shared>, yet the engine and tests still hold it in Arc<Pager>"
tags:
  - quality
---

# Feature: one way to share a pager

## Report

`Pager` became `Pager<R>(Arc<Shared<R>>)` with `Clone` and `Deref<Target = Shared<R>>` (crates/adtb/src/storage/pager.rs:130-147), but every holder that shares it still wraps it in another `Arc`: `write::Database` and `Setup` (crates/adtb/src/write/mod.rs:227, 259, and `pager()` at 349), `api::Engine` (crates/adtb/src/api/mod.rs:127, built at 285), `exec::Store` (crates/adtb/src/exec/store.rs:63, 152), and the threaded tests in crates/adtb-sim/tests (btree_threads.rs:283, 461, 682; heap_threads.rs:249; pager.rs:303; commit.rs:93, 114, 181, 342). `Shared` is pub only to be the `Deref` target.

Fix: pick one. Either drop `Arc<Pager>` everywhere and clone the handle, or revert `Pager` to a plain struct shared through `Arc`.
