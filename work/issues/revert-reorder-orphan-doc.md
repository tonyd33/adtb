---
created-at: 2026-10-27
status: in-progress
kind: bug
priority: 896
summary: "reorder's doc comment was left above grants_back's, so grants_back carries two docs and reorder none"
branch: quality-fixes30
tags:
  - quality
  - lang
---

# reorder's doc comment sits above grants_back's

## Report

The doc comment for `reorder` at `crates/adtb-lang/src/evolve/revert.rs:1057` was left above `grants_back`'s own doc, so `grants_back` carries two docs and `reorder` (`revert.rs:1097`) none. Same pattern as `catalog-orphan-doc-comment` and `reference-upsert-doc-misplaced`.

Fix: move it back above `reorder`.
