---
created-at: 2026-10-31
status: open
kind: feature
priority: 405
summary: separate MAX limits stand in for one budget and sized stacks
---

# Feature: One spend budget

## Request

## Proposal

## Open questions

Roles-adhoc found four separate causes of unbounded work and answers them with separate limits, including `MAX_RUN_HEIGHT` 64, which fits today's stack and states no rule.

Replace them with one budget charged across check, normalise, analyse and plan, plus sized worker stacks and sharing-aware lowering. Add a scaling test over shared shapes at n and 2n, per pass, asserting a work ratio.

Dissolves checker-wide-types, kernel-depth-audit, part of kernel-path-sharing. Costly in plan and exec, so it follows the other four.
