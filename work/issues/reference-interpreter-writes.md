---
created-at: 2026-10-25
status: open
kind: feature
priority: 220
summary: "adtb-reference cannot evaluate writes, change sets, delete actions, upsert, retag or transact blocks"
depends-on: [reference-interpreter-queries, kernel-writes-contract]
tags: [reference]
---

# Feature: reference interpreter for writes

## Report

`adtb-reference` evaluates no write. Milestone 2 of `docs/plan.md` needs
the equations of `docs/spec/semantics.md` §6 over the kernel forms that
`kernel-writes-contract.md` states:

- change sets: `ins`, `del`, `rep` and `kill` effects applied as one
  multiple assignment;
- delete actions (`clear`, `cascade`, `retag`) per §6.4;
- upsert, on the key and on a unique;
- retag, as assignment of a value of another constructor;
- `transact` blocks, `let` bindings over writes, `rejects into`, `fail`
  and `reject`;
- naive constraint checking: every denial query evaluated at each
  statement boundary.
