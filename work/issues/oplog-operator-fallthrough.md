---
created-at: 2026-10-26
status: open
kind: feature
priority: 891
summary: "the operator log's encoder splits one match over Resolved into three chained catch-alls, so a new operator compiles and panics at runtime"
tags:
  - quality
---

# Feature: one exhaustive match encodes each resolved operator

## Report

`operator` (crates/adtb/src/catalog/oplog.rs:125) ends in `other => changing(schema, ty, other)` (oplog.rs:188), `changing` ends in `other => reshaping(schema, ty, other)` (oplog.rs:283), and `reshaping` ends in `declaring => panic!("{declaring:?} is a declaring operator")` (oplog.rs:345). The three functions together cover every `Resolved` variant, but no single match is exhaustive, so a variant added to `Resolved` compiles and panics on the first migration that logs it. The panic message is also wrong for that case: the declaring operators are handled in `operator` and never reach `reshaping`.

Fix: keep one exhaustive match over `Resolved` with no wildcard arm, whose arms call a helper per group with the destructured fields, or group the variants into sub-enums in `adtb_lang::evolve` so each helper matches its own group exhaustively.
