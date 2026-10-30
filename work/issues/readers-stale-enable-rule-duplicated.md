---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 895
summary: "Catalog::stale restates the readers-first enable rule that enabling and enabling_value apply, so registration and migration can disagree on what a reader blocks"
closed: 2026-10-29
tags:
  - quality
  - engine
---

# Catalog::stale restates the readers-first enable rule

## Report

`Catalog::stale` (`crates/adtb/src/catalog/readers.rs:277-317`) reimplements inline the rule `enabling` (`readers.rs:431`), `enabling_value` (`:460`) and `blocking` (`:481`) apply: whole reads, followed references, `hides_new` and `reaches`. The copies already differ in shape (`stale` skips `Stage::Declared` itself). A change to one makes registration's `BundleStale` disagree with migration's refusal about what a pinned reader blocks.

Fix: one predicate, "does pinned reader `p` block constructor C in schema `s`", called from both sides.

## 2026-10-29: closed

Fixed by track quality-fixes30, landed at a3e00f73. Receipt: work/briefs/receipt-quality-fixes30.md. Branch check 2832 passed, 7 skipped, campaign 47.

## 2026-10-29: resolved
