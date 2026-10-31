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

Measured by roles-adhoc round 4, all still unbounded by the limits: a role's list of about 720000 literals passes 1.5 GiB of heap from about 2 MB of text; a text at `MAX_STEPS` holds the permission check about 4 s; a list of about 600000 additions takes 12 s before its refusal; a chain ending a root is not measured by `Budget::spend`, so 4000 literal query arms normalise and only the run bound refuses them. These must be bounded before a socket accepts text.

From the roles-adhoc delta review. An existing database holding a body past a bound fails its open as corrupt (api/mod.rs:875,1031), so lowering a limit makes a file unopenable: a stored body must not be judged by a limit at open. The type node charge refuses only ad hoc text, so registered and schema text walk types with no cap. Height refusal depends on wrap order: bare `match n` with 4000 write arms passes check and the engine refuses it at the run bound while the reference runs it, and `transact` refuses at 2000. Statements charge unevenly: 65 `let x = insert` refused, 1000 plain inserts run.
