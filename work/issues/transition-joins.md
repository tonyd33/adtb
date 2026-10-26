---
created-at: 2026-10-25
status: open
kind: feature
priority: 743
summary: "a delta joining two change-set inputs runs as a nested loop, quadratic in the change size"
tags: [engine]
---

# Feature: hash joins of change-set inputs

## Report

In `write/delta.rs`, a delta that reads two transition inputs joins them with nested loops, O(|Δ|²). Plan them as a hash join on the connecting equality.
