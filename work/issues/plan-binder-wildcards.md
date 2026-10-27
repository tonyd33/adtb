---
created-at: 2026-10-26
status: open
kind: feature
priority: 794
summary: "the planner's with_vars and bound match Kind with a wildcard, so a new binder is silently not renamed or scoped"
tags:
  - quality
---

# Feature: exhaustive binder matches in the planner

## Report

`crates/adtb/src/plan/terms.rs` `with_vars` (terms.rs:130, new in this range) and `bound` (terms.rs:162) list the binders of `Kind` and end in `_ => {}`. They are complete today, but a binder added to `Kind` compiles without either knowing it: `with_vars` would leave it unrenamed, so the alpha-equivalence test in `crates/adtb/src/plan/lower.rs:1011` compares wrong, and `bound` would leave its body's variables free. Both encode facts about the kernel's binders that belong with `Kind` in `crates/adtb-lang/src/kernel.rs`, beside `children` and `with_children`.

Fix: move both to `Kind` in kernel.rs with exhaustive matches that name every variant, as `Kind::children` does.
