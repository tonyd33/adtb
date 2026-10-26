---
created-at: 2026-10-25
status: open
kind: bug
priority: 597
summary: "a migration refuses a path index the kernel promises"
tags: [quality]
---

# Bug: migration refuses path indexes

## Report

catalog/rows.rs accepts only bare-name index fields and catalog/mod.rs refuses path indexes on migration, although the kernel states them and write/eval.rs maintains them.
