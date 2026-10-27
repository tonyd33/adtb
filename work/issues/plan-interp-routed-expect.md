---
created-at: 2026-10-27
status: open
kind: feature
priority: 870
summary: "the plan test interpreter's routed carries a too_many_arguments expectation whose reason lists the arguments instead of what would remove it"
tags: [quality, test]
---

# Feature: pass the index end of routed as one value

## Report

`routed` at `crates/adtb/src/plan/tests/interp.rs:718-721` has `#[expect(clippy::too_many_arguments, reason = "an index end's parts and the evaluation it runs in")]`. The reason names what the arguments are, not what would make the expectation go. The executor groups the same parts as `IndexEnd<'a>` in `crates/adtb/src/exec/run.rs`.

Fix: take a struct like `IndexEnd` and drop the expectation.
