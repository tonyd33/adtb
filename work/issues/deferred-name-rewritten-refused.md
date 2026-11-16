---
created-at: 2026-11-16
status: open
kind: bug
priority: 300
summary: a migration is refused when a later operator rewrites a declaration whose names wait on a relation or set added later
tags:
  - lang
  - class-identity-by-spelling
---

# Deferred name rewritten refused

## Report

Found by the review of committed-schema-type part 2b-ii (2026-11-16). A declaration whose names wait for a relation or named set added later in the same migration, rewritten by a later operator before that, is refused "a later operator of the migration rewrites the declaration this operand was written into, so its names do not resolve" (crates/adtb-lang/src/evolve/names.rs:160-165, the range lands in `missing`). Valid migrations:

- `add type S = { id: Key ss, z: Option (Key zs) }`, `add relation ss`, `drop relation ss`, `drop type S`, then `add relation zs`: refused (main refuses it twice).
- `define fn f (k: Key ws.Settled) -> Key ws = k`, `redefine fn f` (same), then `add set Settled of ws`: refused (main panics at canon.rs:202).

Fix direction: a deferred range whose declaration a later operator rewrites resolves in the source of the operator that rewrote it, or is dropped when no stored text holds it. A searching test over add, rewrite and drop sequences of a waiting declaration.
