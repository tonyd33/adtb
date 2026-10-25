---
created-at: 2026-10-25
status: open
kind: feature
priority: 260
summary: "matches and collate build a new regex or collator on every call"
tags:
  - reference
---

# Feature: Cache compiled regexes and collators

## Report

In `adtb-reference`, `matches` and `collate` compile their constant
argument on every call, which is slow inside a loop. Their arguments
are compile-time constants, so compile each one once per statement.
