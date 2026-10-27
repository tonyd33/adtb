---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 699
summary: "the engine refuses constraints that join through an optional reference, use paths inside match arms, or follow two references, though the spec admits them"
closed: 2026-10-26
tags:
  - engine
  - constraints
---

# Bug: Constraint admission refuses admitted path forms

## Report

Found by the campaign manager (`apps/campaign/docs/gaps.md`, "Engine: admission"):
- A constraint joining through `Option (Key r.C)`, as in `where q.giver == Some { value: key c } and c.campaign != q.campaign`, is refused as not admitted, so release 1.0 of the app does not open (`campaign::engine release_1_0_opens`).
- Paths inside the arms of a `match` over a sum are refused.
- A path through two references is refused, while the same joins written out are admitted.

Each is index-bounded the same way its written-out join is. The fix belongs in the reference index and admission analysis; each form gets an admission test and a conformance case.

## 2026-10-26: landed

Landed in `feat(constraints): admit joins through optional references, paths in match arms and paths of several references with index-bounded checks`. The planner splits joins on constructor equalities in `plan/lower/split.rs`, and the delta compiler unfolds dereferences and keeps arm conditions. The app's two constraints (`quest_giver_in_campaign`, `location_parent_in_campaign`) are restored, and `release_1_0_opens` passes.

A top-level match arm still drops its scrutinee's conditions; filed separately as `admission-top-match-arm`.

## 2026-10-26: resolved
