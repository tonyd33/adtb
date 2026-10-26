---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 260
summary: matches and collate build a new regex or collator on every call
closed: 2026-10-25
tags:
  - reference
---

# Feature: Cache compiled regexes and collators

## Report

In `adtb-reference`, `matches` and `collate` compile their constant
argument on every call, which is slow inside a loop. Their arguments
are compile-time constants, so compile each one once per statement.

## 2026-10-25: resolved

Patterns and collators are cached per statement by their text.
