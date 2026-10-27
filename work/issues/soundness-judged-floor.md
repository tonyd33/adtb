---
created-at: 2026-10-26
status: open
kind: feature
priority: 778
summary: "the soundness sweeps pass with one judged item per source, and count internal aborts as judged"
tags:
  - quality
  - test
---

# Feature: the soundness sweeps assert a floor on how many items they judge

## Report

`Report::assert` (`crates/adtb-testkit/tests/soundness.rs:68-76`, and :519) requires only `judged > 0` and `checked > 0`. If nearly every generated, fuzzed or conformance statement began aborting (`NotYet`, a rejection) or failing to check, all four tests would pass with one judgement each. `internal()` (soundness.rs:153) counts an internal abort as judged.

Fix: assert a share of judged items per source, as differential.rs does with its nonempty and form floors, and count aborts apart from judgements.
