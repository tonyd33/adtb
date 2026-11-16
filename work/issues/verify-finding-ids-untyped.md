---
created-at: 2026-11-16
status: open
kind: bug
priority: 870
summary: "verify findings carry catalog relation, constraint, index and constructor ids as raw integers and a relation scope as formatted text parsed back"
tags:
  - quality
  - engine
  - class-identity-by-spelling
---

# Verify finding ids untyped

## Report

Found by the quality review of reviewed..cdb9f9f5. The verify path drops the typed catalog ids at each boundary and rebuilds them by hand:

- `write::Found` holds `index`, `ctor`, `relation` and `constraint` as `u64` (crates/adtb/src/write/verify.rs:61-76), so a relation id passed where a constructor id belongs compiles.
- `FindingRow::violated` returns `Option<u64>` (crates/adtb/src/api/verify.rs:81-87), and its one caller rewraps it with `CatConstraintId::from_raw` (crates/adtb/src/api/command.rs:695).
- `KeptRow::relation` is a `u64` (crates/adtb/src/catalog/kept.rs:31), compared with `held_id(..).raw()` (api/verify.rs:312, :321, :521), while `RootsRow::HeapRoot` beside it carries a `CatRelId`.
- A relation scope's group is the text `relation {id}` (api/verify.rs:290-295), and `removed` parses the id back out of that text (api/verify.rs:298-303).
- `fields` writes an id past `i64::MAX` as `i64::MAX` (api/verify.rs:226), where the catalog's `int` panics on one.

Each holds today because the values are copied in the right order, not because a type keeps them apart.

Fix direction: `CatRelId`, `CatCtorId`, `CatIndexId` and `CatConstraintId` in `Found`, `FindingRow` and `KeptRow`, converted to raw integers only where a kept row is encoded. Keep the group as a typed scope, and leave its text form to the encoder. Use one conversion for an id that does not fit 63 bits.
