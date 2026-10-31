---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 560
summary: LargeStore::settle latches and commits a stale freed page unchanged
closed: 2026-10-31
tags:
  - quality
  - engine
---

# Feature: skip freed pages in LargeStore::settle

## Report

`crates/adtb/src/heap/large.rs` `settle` latches the pages `touched()` names and commits them, including one a sweep has freed since. It writes nothing there today, so it is harmless, but nothing checks it. Skip a page the store no longer owns, as `insert` now does.

## 2026-10-31: closed by triage

Obsolete. heap/large.rs:464 returns early on a stale page.

## 2026-10-31: resolved
