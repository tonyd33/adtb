---
created-at: 2026-10-29
status: resolved
kind: bug
priority: 380
summary: plan/terms.rs with_children and with_vars duplicate Kind::with_children and Kind::with_vars
closed: 2026-10-29
tags:
  - quality
---

# Plan terms kind duplicates

## Report

Found by the kernel-free-vars track (2026-10-29). `with_children` and `with_vars` in crates/adtb/src/plan/terms.rs copy `Kind::with_children` and `Kind::with_vars` in crates/adtb-lang/src/kernel.rs.

Fix: delete the plan copies and call the kernel ones.

## 2026-10-29: closed

Fixed by track quality-fixes30, landed at a3e00f73. Receipt: work/briefs/receipt-quality-fixes30.md. Branch check 2832 passed, 7 skipped, campaign 47.

## 2026-10-29: resolved
