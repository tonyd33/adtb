---
created-at: 2026-10-25
status: open
kind: feature
priority: 560
summary: "LargeStore::settle latches and commits a stale freed page unchanged"
tags: [quality, engine]
---

# Feature: skip freed pages in LargeStore::settle

## Report

`crates/adtb/src/heap/large.rs` `settle` latches the pages `touched()` names and commits them, including one a sweep has freed since. It writes nothing there today, so it is harmless, but nothing checks it. Skip a page the store no longer owns, as `insert` now does.
