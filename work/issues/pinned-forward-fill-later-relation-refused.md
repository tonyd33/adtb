---
created-at: 2026-11-14
status: resolved
closed: 2026-11-16
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

## Resolved

Landed as pkrpqpxz e006c7e8 (committed-schema-type part 4b, 2 rounds, full check 0, 3839 tests). Translation replays a logged migration with `expecting`; a fill's key of a relation a later operator adds resolves through its adding operator (`Adder::Operator(k)`, the `Origin::Logged(i, k)` op's `Resolved::AddRelation`) to the relation at the current version. If a later migration dropped that relation (reachable through a function parameter), the query is refused "it reaches `NAME`, which the current version lacks" under the fill's operator; a reverted fill's key of a relation its revert restores later is an invariant. Script bundles/fill-comparing-a-key-of-a-relation-added-later.test; translate tests for the insert and the dropped-since refusal.
