---
created-at: 2026-10-25
status: open
kind: bug
priority: 179
summary: reindex r index (f) in S does not check that such an index is declared
tags:
  - lang
---

# Bug: reindex accepts an index that does not exist

## Report

Index fields are stored as span-carrying AST with no span-insensitive comparison, so `reindex r index (f) in S` resolves `r` and type checks `f` but never matches it against a declared index.
