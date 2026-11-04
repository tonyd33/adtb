---
created-at: 2026-11-03
status: open
kind: feature
priority: 650
summary: "VacuumBudget.cpu has no measure: background maintenance charges wall time to both fractions, so the effective budget is min(io, cpu)"
---

# Feature: Measure CPU for the vacuum budget

## Request

## Proposal

## Open questions

From the background-vacuum plan. The runtime has no per-thread CPU clock and the simulator models no CPU time, so the shared pacer charges wall time to `io` and `cpu` alike. Give the runtime a per-thread CPU measure (and the simulator a model of it), then charge each fraction its own time.
