---
created-at: 2026-10-25
status: open
kind: feature
priority: 870
summary: "no history heaps, time-travel window or dropping of a removed constructor's heap"
depends-on:
  - vacuum
tags:
  - engine
---

# Feature: History heaps and removed constructors

## Report

Not built: history heaps and the time-travel window
(`docs/design/mvcc.md` §10.3), and dropping a removed constructor's
heap (§10.4). Holes in a heap's directory also grow by one per removed
page until an insert fills one. Bound that too.
