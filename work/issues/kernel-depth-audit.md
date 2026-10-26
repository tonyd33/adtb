---
created-at: 2026-10-26
status: open
kind: feature
priority: 799
summary: "the planner, the delta compiler and the reference evaluator recurse over kernel depth and were never audited against the front end's nesting limit"
tags:
  - quality
---

# Feature: Kernel depth audit

## Request

## Proposal

## Open questions

## Report

The front end now bounds parse depth at 64 levels, but passes outside `adtb-lang` (the planner in `crates/adtb/src/plan`, the delta compiler in `crates/adtb/src/write/delta`, the reference evaluator) recurse over kernel terms. Run the front end fuzzer's cases through them at the limit, fix what overflows, and switch `crates/adtb/src/plan/terms.rs` `with_children` to `Kind::with_children` in `crates/adtb-lang/src/kernel.rs`, which now duplicates it.
