---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 220
summary: "adtb-reference cannot evaluate writes, change sets, delete actions, upsert, retag or transact blocks"
closed: 2026-10-25
depends-on:
  - reference-interpreter-queries
  - kernel-writes-contract
tags:
  - reference
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

## 2026-10-25: done, awaiting landing

The work is done on `refw`: 65 of the 69 new tests pass. The other 4
fail on `norm-fresh-let-substitution.md` and
`unique-value-level-path.md`. The constraint naming rulings it raised
are `spec-constraint-naming-gaps.md`. Closes when it lands.

## 2026-10-25: resolved

Landed as `feat(reference): evaluate writes, blocks and statement boundaries`. The four failing tests wait on `norm-fresh-let-substitution.md` and `unique-value-level-path.md`.
