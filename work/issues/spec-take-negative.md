---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 288
summary: take n with a negative n is unspecified
closed: 2026-10-25
tags:
  - spec
---

# Feature: take with a negative count

## Report

Found by the reference interpreter. The spec does not say what `take n`
does when `n` is negative.

Ruling: it faults with `NegativeCount`, as `top` does.

## 2026-10-25: resolved

In the spec with `docs: record impl4 rulings`.
