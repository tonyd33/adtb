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

## 2026-10-26: chosen

Tony left the choice to the lead. The first application is Partcrate (working name, trademark and domain unchecked), a local-first desktop app for electronics parts inventory and small-batch builds, sold as a one-time licence. Parts, lots, builds and orders are relation-level sums; reservations reference `builds.Kitting`; aggregate denial constraints bound reserved quantities; releases 1.1 to 1.3 ship as typed migrations. The stress workload is a simulated contract assembler. The market study, scope and risks are in `work/app/choice.md`.
