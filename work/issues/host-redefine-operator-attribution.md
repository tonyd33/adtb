---
created-at: 2026-11-04
status: open
kind: bug
priority: 560
summary: A host redefine refused under a live pin names another operator of the migration
---

# Host redefine refusal names the wrong operator

## Report

A migration holding `add type T = A | B` and `redefine fn f`, refused under a live pin because its migration does not resolve, names `operator: "add type T = A | B"` in the refusal. The change interop.md refuses is the redefine, so the operator named should be `redefine fn f`. The Rust test `a_host_redefine_under_a_live_pin_is_refused_when_its_migration_does_not_resolve` in crates/adtb-sim/tests/sim/bundles.rs compares only the detail and so does not see it.

Found in the review of the roles and server script batch, 2026-11-04.

## Proposal

Attribute the refusal to the operator whose change the pin cannot follow, and make that test compare the whole refusal.
