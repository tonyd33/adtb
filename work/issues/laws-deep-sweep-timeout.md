---
created-at: 2026-10-27
status: open
kind: feature
priority: 881
summary: "the ignored deep law sweep every_operator_keeps_its_laws times out at 60 s with ADTB_DIFF_CASES=8"
tags:
  - test
---

# Feature: make the deep law sweep runnable

## Report

`every_operator_keeps_its_laws` in `crates/adtb-testkit/tests/differential.rs` is ignored and draws every operator over long walks, but with `ADTB_DIFF_CASES=8` it passes the 60 s timeout, so the operators taken off the skip lists have not been checked over long walks. Each open that applies a revert takes about 2 s (see `migration-open-slow`). Give the deep sweep a nextest profile with a longer timeout, and run it.
