---
created-at: 2026-11-14
status: open
kind: bug
priority: 300
summary: "a bundle pinned before a migration whose fill reads a key of a relation a later operator adds is refused at open, because translation replays the steps without the later relations"
tags:
  - engine
  - class-failure-flattened
---

# A pinned bundle across a fill naming a later relation is refused at open

## Report

A bundle pins version 1; migration 2 is `add type Z = { id: Key zs }`, `add field O.x : Text fill match | O o -> print(o.id)`, `add relation zs`. The open is refused with `RegistrationRefused ... "across `add field O.x : Text fill match`: no `zs` in scope"`, on main and since. Cause: `replayed` in crates/adtb-lang/src/translate/mod.rs replays the migration op by op without `expecting`, so the step before `add relation zs` does not build, although the migration applied. Found in the review of the ahead-declared track; reproduced as an engine script, not kept.

Direction: replay each step with the migration's ahead relations declared (as the forward apply does), then read translated values through the 6b rule (a key of a relation a later operator adds reaches nothing). Test: the history above with a bundle whose callable writes `os`, expecting it served.

Defect class: Damage or failure gives a panic, hang or flattened error.
