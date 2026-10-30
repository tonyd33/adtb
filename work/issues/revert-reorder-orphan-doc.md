---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 896
summary: "reorder's doc comment was left above grants_back's, so grants_back carries two docs and reorder none"
closed: 2026-10-29
tags:
  - quality
  - lang
---

# reorder's doc comment sits above grants_back's

## Report

The doc comment for `reorder` at `crates/adtb-lang/src/evolve/revert.rs:1057` was left above `grants_back`'s own doc, so `grants_back` carries two docs and `reorder` (`revert.rs:1097`) none. Same pattern as `catalog-orphan-doc-comment` and `reference-upsert-doc-misplaced`.

Fix: move it back above `reorder`.

## 2026-10-29: closed

Fixed by track quality-fixes30, landed at a3e00f73. Receipt: work/briefs/receipt-quality-fixes30.md. Branch check 2832 passed, 7 skipped, campaign 47.

Already fixed at 6320bf00 when the track reached it.

## 2026-10-29: resolved
