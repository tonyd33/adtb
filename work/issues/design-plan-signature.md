---
created-at: 2026-10-26
status: open
kind: bug
priority: 773
summary: "execution.md's plan call omits the normal program parameter the planner takes"
tags:
  - quality
  - design
---

# execution.md states a plan signature without the normal program the planner takes

## Report

`docs/design/execution.md`, the planner's input, gives the call as `plan::plan(schema, catalog, terms, root, params, options)`, but `plan` (`crates/adtb/src/plan/lower/mod.rs:31-38`) also takes `normal: &NormalProgram`. A caller reading only the design file cannot call it.

Fix: add the parameter to the documented call and state what the planner reads from it.
