---
created-at: 2026-11-17
status: open
kind: bug
priority: 500
summary: "evolution.md 3.2 and 3.4 do not state that redefine fn rebuilds dependent indexes and revalidates dependent constraints, which interop.md 9.5 promises"
tags:
  - spec
  - class-none
---

# Evolution redefine dependents unstated

## Report

Found by fn-dependents (2026-11-17). interop.md §9.5 states that `redefine fn` rebuilds every index and stored view whose expression calls the function and rechecks every constraint and registered query, and evolution.md §6.4 and §7.1 say indexes and constraints always use the current definitions; evolution.md §3.2 and §3.4, where the operators are defined, say nothing of it. State it at the operator, through the spec-revision workflow.
