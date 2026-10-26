---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 292
summary: "the catalog breaks the model's reference rule, catalog::audit is undeclared, and catalog keys and evolution corner cases are unstated"
closed: 2026-10-25
tags:
  - spec
---

# Bug: Catalog and evolution gaps

## Report

Found checking the engine's own declarations and the migration
operators. My rulings:

1. `docs/spec/catalog.md` §2.4 puts references inside value-level sums
   (`Operator`, `ClauseBody`, `CtorTarget`, `Place`, `ArmTarget`,
   `MigrationOp.op`), which `model.md` §8.2 forbids. The model rule
   stays absolute. Those fields hold the target's key value, not a
   `Key`. The engine keeps their integrity, and a reader joins on them.
   Remove the exemption in `crates/adtb-lang/src/schema.rs`
   `check_references` once the catalog is fixed.
2. `catalog::audit` is declared `relation audit : AuditEvent key seq`.
3. `.key(n)` of a catalog or sys relation has that relation's declared
   key type: `Bytes` for `catalog::wasm_modules`, `I64` where the key
   is a catalog id. State it in catalog.md.
4. A relation added after its type gained a constructor takes the
   type-level write state for that constructor. State it in
   evolution.md.
5. `add constructor` on a record type is allowed, as on any sum: a
   record is a one-constructor sum. Registered queries are rechecked,
   and the migration is refused if one fails. State it in evolution.md;
   the checker refuses it today (`evolve-record-add-ctor.md`).

## 2026-10-25: resolved

Landed as `feat(lang): give schemas catalog ids and constructor stages and fix the catalog's references`. Rulings in point `spec3`, for Tony to adopt.
