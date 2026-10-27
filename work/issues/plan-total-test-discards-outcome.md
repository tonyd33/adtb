---
created-at: 2026-10-27
status: open
kind: bug
priority: 871
summary: "an_existence_test_over_total_pipelines_is_marked_total discards the executor's outcome, so it never checks the result against the reference"
tags: [quality, test]
---

# The total-pipeline existence test runs the query and throws the result away

## Report

`an_existence_test_over_total_pipelines_is_marked_total` in `crates/adtb/src/plan/tests/mod.rs` calls `outcome(&f, text, &[]).ok();`, discarding the result. It asserts the plan is marked total but never that the executor's answer agrees with the reference, nor that the partial cases abort.

Fix: `agrees(&f, text, &[])` for the total cases and an assertion of the expected abort for the partial ones.
