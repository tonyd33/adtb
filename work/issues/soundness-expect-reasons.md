---
created-at: 2026-10-26
status: open
kind: bug
priority: 779
summary: "soundness.rs's dead_code and disallowed_methods expectations name why, not what would remove them"
tags:
  - quality
  - test
---

# soundness.rs's expect reasons do not say what would remove them

## Report

- `crates/adtb-testkit/tests/soundness.rs:480-483`: the `clippy::disallowed_methods` reason, "the fuzzed statements need a deeper stack than a test thread's", says why, not what removes it: the reference evaluator no longer recursing per match arm (`reference-eval-recursion.md`).
- soundness.rs:4-7: the `dead_code` reason names the unused part of the shared `running` module but not the condition for dropping it.

Fix: phrase both reasons as the condition under which the expectation goes.
