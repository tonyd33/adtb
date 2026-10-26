---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 717
summary: a plan does not run a user-defined aggregate
closed: 2026-10-26
---

# Feature: User aggregate plan

## Request

## Proposal

## Open questions

## Report

Conformance file queries/10-user-aggregate fails: the planner has no plan for a user-defined aggregate.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as ryzyykky: `plan::Sink::Fold`, folded sequentially. The aggregate's `merge` part is unused, and user aggregates skip the fused group-aggregate rewrite; both are in planner-follow-ups.
