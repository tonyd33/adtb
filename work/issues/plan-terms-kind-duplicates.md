---
created-at: 2026-10-29
status: open
kind: bug
priority: 380
summary: "plan/terms.rs with_children and with_vars duplicate Kind::with_children and Kind::with_vars"
tags:
  - quality
---

# Plan terms kind duplicates

## Report

Found by the kernel-free-vars track (2026-10-29). `with_children` and `with_vars` in crates/adtb/src/plan/terms.rs copy `Kind::with_children` and `Kind::with_vars` in crates/adtb-lang/src/kernel.rs.

Fix: delete the plan copies and call the kernel ones.
