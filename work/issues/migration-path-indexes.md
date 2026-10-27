---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 597
summary: a migration refuses a path index the kernel promises
closed: 2026-10-26
tags:
  - quality
---

# Bug: migration refuses path indexes

## Report

catalog/rows.rs accepts only bare-name index fields and catalog/mod.rs refuses path indexes on migration, although the kernel states them and write/eval.rs maintains them.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as mrtsvsxs.
