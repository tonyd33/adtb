---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 598
summary: a checker bug reaching the engine or the reference panics the process
closed: 2026-10-25
tags:
  - quality
---

# Feature: internal errors instead of runtime type panics

## Report

exec/prim.rs, adtb-reference prim.rs and eval.rs, and plan/lower.rs panic on values or normal forms the checker should have ruled out (for example `other => panic!("expected text, found {other:?}")`). Give these paths an internal-error variant that fails the statement and names the broken invariant.

## 2026-10-25: resolved

## 2026-10-25: resolved

`exec::Error::Internal`, `write::Error::Internal`, `plan::Error::Internal` and `adtb_reference::Abort::Internal` carry a message naming the broken invariant. The statement fails and the transaction stays open; `try` and checked primitives do not absorb it. Left as panics: the reference `Val` accessors (`val-accessors-fallible`), stored-data conversion, catalog and schema-shape expects. The conflict with the fatal-split ruling and the missing client error are `checker-bug-errors`.
