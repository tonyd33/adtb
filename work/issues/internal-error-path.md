---
created-at: 2026-10-25
status: open
kind: feature
priority: 598
summary: "a checker bug reaching the engine or the reference panics the process"
tags: [quality]
---

# Feature: internal errors instead of runtime type panics

## Report

exec/prim.rs, adtb-reference prim.rs and eval.rs, and plan/lower.rs panic on values or normal forms the checker should have ruled out (for example `other => panic!("expected text, found {other:?}")`). Give these paths an internal-error variant that fails the statement and names the broken invariant.
