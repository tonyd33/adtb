---
created-at: 2026-10-30
status: in-progress
branch: quality-fixes33
kind: bug
priority: 899
summary: "the doc comment of Stmt::probe sits on stopped, so stopped carries two docs and probe none"
tags:
  - quality
  - engine
---

# Write probe doc misplaced

## Report

`crates/adtb/src/write/eval.rs:949` has `Returns the entries of index id as Store::probe does, bounded by the statement.` above `stopped`, followed by that function's own line. The `probe` it describes, a few lines down, has no doc.

Fix direction: move the first line onto `probe`.
