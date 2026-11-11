---
created-at: 2026-11-11
status: open
kind: bug
priority: 450
summary: "the plan validator decides a case's constructor count and set with its own copy of exec::types' rule, so a divergence passes a plan whose arm table exec indexes out of range, or refuses a valid one"
tags:
  - quality
  - engine
  - class-copied-rules
---

# The plan validator and the executor decide a case's constructors by two copies of one rule

## Report

`plan::validate`'s `case_ctors` and `all_of` (crates/adtb/src/plan/validate.rs:437-462) unwrap newtypes, count a `Data` or `Sum` type's constructors, take its selected set and recurse through `Tag`. `exec::types::repr`, `ctors` and `ctor_set` (crates/adtb/src/exec/types.rs:8-55) do the same separately.

`exec::ir::Lower::arms` (exec/ir.rs:369-380) sizes its arm table from `exec::types::ctors` and indexes `out[*c as usize]` with no check, relying on the validator's `ArmNotCtor` refusal. If the two copies diverge (a new `Ty` form, a change to newtype handling in one of them), the validator passes a plan that panics in exec's lowering, or refuses a valid one. The newtype unwrap is copied at further sites: catalog/rows.rs, catalog/layout.rs, api/value.rs, write/check.rs.

Direction: one function returning a type's constructor count and constructor set, called by the validator and by exec's lowering; a test that the validator's accepted arms are exactly the indexes exec's table has, over the planner corpus.

Defect class: Wildcards and copied rules.
