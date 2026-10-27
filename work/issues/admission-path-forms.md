---
created-at: 2026-10-26
status: open
kind: bug
priority: 699
summary: "the engine refuses constraints that join through an optional reference, use paths inside match arms, or follow two references, though the spec admits them"
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
