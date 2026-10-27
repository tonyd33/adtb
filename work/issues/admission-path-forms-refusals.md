---
created-at: 2026-10-26
status: open
kind: feature
priority: 762
summary: the newly admitted constraint path forms have no refused near miss in the sim tests or conformance
tags:
  - quality
  - test
---

# Feature: refused near misses for the newly admitted constraint path forms

## Report

The constraint forms admitted in 86e5727 (a join through an optional reference, paths in match arms, a path through several references) are tested only for admission and agreement (`crates/adtb-sim/tests/constraints/paths.rs:105-131`, `conformance/constraints/05-admitted-optional-reference-join.test`, `05-admitted-match-arm-paths.test`, `05-admitted-two-reference-path.test`). No near miss is pinned as still refused, such as the same optional join correlated on a field that is not a reference, or an arm path whose field has no implied index, so a widening of admission past the index bound would pass.

Fix: add one refused counterpart per form through `refused()` in `crates/adtb-sim/tests/constraints.rs`, and the same in conformance.
