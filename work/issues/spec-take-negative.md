---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 288
summary: take n with a negative n is unspecified
branch: default
tags:
  - spec
---

# Feature: take with a negative count

## Report

Found by the reference interpreter. The spec does not say what `take n`
does when `n` is negative.

Ruling: it faults with `NegativeCount`, as `top` does.
