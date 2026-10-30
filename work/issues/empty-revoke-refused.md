---
created-at: 2026-10-29
status: resolved
kind: bug
priority: 400
summary: "the checker accepts a revoke that matches no held grant, which spec/empty-revoke/refuse refuses"
closed: 2026-10-30
---

# Empty revoke refused

## Report

Ruling spec/empty-revoke/refuse (2026-10-29): a migration revoke that matches no held grant, including a listed revoke whose field list matches no held field set, refuses the migration naming the revoke. Today the checker accepts it (spec-evolve-gaps, grant-carry-follow-ups). Fix in crates/adtb-lang/src/evolve/grant.rs `revoke`, with a test per form.

## 2026-10-30: landed

Landed as 0c1fc305. Full check passed: 2923 tests, 7 skipped; campaign 52. An existing log with an
empty revoke now fails to open, and an old `version_hash` is not recomputed; both accepted before
release. A capability is given by a `schema` holder or a holder of it with grant option.

## 2026-10-30: resolved
