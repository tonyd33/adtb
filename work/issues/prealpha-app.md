---
created-at: 2026-10-25
status: open
kind: feature
priority: 850
summary: no application exercises adtb end to end
depends-on:
  - milestone-7-engine
tags:
  - prealpha
  - test
---

# Feature: A first application on the pre-alpha

## Report

An application, chosen by Tony, written against the pre-alpha API: its schema, registered queries and transactions as ad hoc text for now, and a workload for the load harness. Gaps it finds in the language, model or API become tickets or design-space questions.

## 2026-10-25: deferred

Tony: build the application once the engine is ready, not earlier. It waits for milestone 8 (embedded alpha); the interpreter-backed pre-alpha tickets are closed.

## 2026-10-25: gate

Tony: the goal is a real application on adtb, stress tested under
simulated load. The work starts when milestone 7 exits: the engine
passes the single-version suite with the differential and partition
tests (`milestone-7-engine.md`). Milestone 8's evolution is then proven
on this application: its first schema changes are the first real
migrations. There is no interpreter-backed pre-alpha.
