---
created-at: 2026-10-25
status: open
kind: bug
priority: 99
summary: "parse, check and normalise overflow the stack on a list literal of about 100 records in debug builds"
tags: [lang]
---

# Bug: stack overflow on a long literal

## Report

From the constraint track: an insert whose value is a list literal of about 100 records overflows the stack in debug builds somewhere in parse, check or normalise; the large-load test in `crates/adtb-sim/tests/constraints.rs` works around it with query inserts. Find the recursion that is linear in the literal's length and make it iterative.
