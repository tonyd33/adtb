---
created-at: 2026-10-27
status: open
kind: bug
priority: 799
summary: "two identical calls of a volatile host function at different call sites of one query may be hash-consed into one evaluation"
tags:
  - lang
---

# Bug: identical volatile calls may share one evaluation

## Report

The normaliser now treats a call of a non-deterministic host function as a draw, so one call site is evaluated once per call. Two call sites with the same text in one query may still be hash-consed into one term and evaluated once instead of twice. Write a test with two such call sites and check each is drawn on its own.
