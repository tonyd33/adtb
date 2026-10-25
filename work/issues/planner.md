---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 715
summary: no planner turns a normalised kernel term into a physical plan
branch: adtb-planner
tags:
  - engine
---

# Feature: planner from kernel terms

## Report

Implement `docs/design/execution.md` §1, §3, §4, §7.2 and §7.4: plans
from normalised kernel terms (`docs/design/kernel.md` §1 and §7) with
constructor-relation scans, union index probes routed by constructor,
one pipeline per `match` arm, joins, grouping, aggregates, semi-naive
`fix`, top-n, pruning, and each rewrite behind a toggle the plan-toggle
tests switch off.

No storage prerequisite: the plan names heaps and indexes by catalog
id.
