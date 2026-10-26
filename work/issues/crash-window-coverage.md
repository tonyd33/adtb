---
created-at: 2026-10-25
status: open
kind: feature
priority: 591
summary: "crash tests that need a crash between two commits of one call may never get one at switch rate 0"
tags: [quality, test, sim]
---

# Feature: audit crash tests for windows switch rate 0 never opens

## Report

The heap prune crash test left 0 of 40 runs unsettled at the default `switch_rate` 0, because without page memory switching both commits always reached the flushed log together; at 0.3, 9 of 40 did. Audit the other crash tests whose interesting window lies between two commits inside one call, give each a switch rate, and assert that the window was reached.
