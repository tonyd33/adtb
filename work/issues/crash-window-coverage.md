---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 591
summary: crash tests that need a crash between two commits of one call may never get one at switch rate 0
closed: 2026-10-25
tags:
  - quality
  - test
  - sim
---

# Feature: audit crash tests for windows switch rate 0 never opens

## Report

The heap prune crash test left 0 of 40 runs unsettled at the default `switch_rate` 0, because without page memory switching both commits always reached the flushed log together; at 0.3, 9 of 40 did. Audit the other crash tests whose interesting window lies between two commits inside one call, give each a switch rate, and assert that the window was reached.

## 2026-10-25: resolved

## 2026-10-25: resolved

btree_threads (a flusher thread, switch rate 0.3) and vacuum_threads (0.3) now assert their window was reached; the heap conversion, isolation and write crash tests assert their counts at rate 0. Landing over the newer main turned up `vacuum-crash-index-mismatch` and `btree-threads-split-window`.
