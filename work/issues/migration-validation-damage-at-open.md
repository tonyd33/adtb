---
created-at: 2026-11-11
status: open
kind: bug
priority: 420
summary: "damage a migration's constraint validation meets at open is refused as MigrationRefused with reason Internal, not as damage"
tags:
  - class-failure-flattened
---

# Damage met validating a migration at open reads as Internal

## Report

## 2026-11-11

When open applies a migration and the migration's constraint validation (catalog/validate.rs `failed`) meets an orphan index entry or decoded damage from the executor (`exec::Error::OrphanEntry`, `exec::Error::Corrupt`), the error is wrapped as `catalog::Broken::Validation(..)` and open replies `MigrationRefused { reason: Internal }`. Damage is never `Internal` (catalog.md §4 keeps `Internal` for checker guarantees). structure-transactions step 8 lifted the same damage to a `Structure` finding for verify's constraint runs and left the open path as on main, because the open rules for damage (a mark, `MigrationRefused` naming `Quarantined`, or `OpenError::Corrupt`) belong to the damage policy and no plant reaches a constraint run during a migration at open.

Direction: decide from the damage policy's open rules what a migration that meets damage in validation replies (and what it marks); make `failed` lift the damage on that path too; a plant that runs between migrations at open, to test it.

Defect class: Damage or failure gives a panic, hang or flattened error.
