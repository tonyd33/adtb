---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 739
summary: "the spec says every constraint is index-bounded, but a reference field needs no index and its target check scans every referrer"
closed: 2026-10-26
tags:
  - spec
  - engine
---

# Feature: references and the index-bounded rule

## Report

`docs/spec/constraints.md` 5.1 makes every constraint index-bounded, references included, yet version 12 of the running example has `identities.user: Key users` with no index on `user`, and the native moved-target check scans every referrer.

My ruling: each reference field has an implied index on the referring field over the referring constructor set, unless a declared index already has that field as its leading field. The engine builds and maintains it like a declared index, and the catalog lists it as implied by the reference. Record it in the design space, state it in `docs/spec/constraints.md` and `docs/spec/catalog.md`, then build it and move the reference checks and delete actions onto index probes.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as yputxooo. The rulings are points `constraint/admit/reference-index/implied` and `constraint/admit/transition-negation/monotone-only`.
