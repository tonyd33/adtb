---
created-at: 2026-10-26
status: in-progress
branch: reference-recursion
kind: bug
priority: 299
summary: "the reference evaluator recurses per match arm and overflows a 2 MB thread on a scalar match with many arms"
tags:
  - reference
---

# Bug: Reference evaluator overflows the stack on wide matches

## Report

Fuzz seed 55 of the soundness test, a scalar `match` with many arms, overflows a 2 MB test thread inside the reference. The soundness fuzz test runs on a 256 MB thread to pass. The evaluator should not need stack proportional to arm count, and its depth should be bounded the way the front end bounds nesting.

## 2026-10-28: dispatched

A fresh writer owns this in workspace reference-recursion on main 1eb25c53, in adtb-reference and the soundness test stack size only: match arms and any other per-element recursion become loops, depth stays bounded by nesting, and FUZZ_STACK drops to the default if seed 55 passes. Gates: suite over adtb-reference and adtb-testkit, clippy, fmt, house-check, size-check. Local brief: work/briefs/brief-reference-recursion.md.
