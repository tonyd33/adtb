---
created-at: 2026-10-29
status: open
kind: bug
priority: 400
summary: "the checker accepts a revoke that matches no held grant, which spec/empty-revoke/refuse refuses"
---

# Empty revoke refused

## Report

Ruling spec/empty-revoke/refuse (2026-10-29): a migration revoke that matches no held grant, including a listed revoke whose field list matches no held field set, refuses the migration naming the revoke. Today the checker accepts it (spec-evolve-gaps, grant-carry-follow-ups). Fix in crates/adtb-lang/src/evolve/grant.rs `revoke`, with a test per form.
