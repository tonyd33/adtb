---
created-at: 2026-11-02
status: open
kind: bug
priority: 140
summary: Translation panics on a tag in a fill past a removal and on a newtype over a sum
---

# Translate boundary panics

## Report

From the review of `fix: name every Kind in the remaining translation analyses`, traced, not run.

- crates/adtb-lang/src/translate/walk.rs `built` calls `p.ctor_name(def, ctor)`, indexing the pin's schema, but inside a fill `def` is the fill schema's. Pin before `U = X | Y` exists; later `add field` on `ts` with `t: Tag U`, fill `X`; then `remove constructor U.X`; a pinned insert into `ts`. Expected a refusal; traced to an out of bounds panic in schema/mod.rs at registration or the migration check. An in range id names the wrong type. The `Data` path has this already; that fix made it reachable for a tag literal.
- crates/adtb-lang/src/translate/map.rs `whole` does not look through a newtype, while `recode` does through `repr`. `query q (u: U) -> U = u`, `U` over `T`, called with `A`, is traced to a panic at crates/adtb/src/api/pinned.rs:41, and a dropped base field at :67. Not confirmed that a newtype over a declared sum is legal.

Also not traced: a stored `Tag T` column or value level sum across `remove constructor`, in heap encoding. Run that first; a wrong answer there outranks both panics.

Over-refusals, safe: `tag r == B` and `[B, C]` at full `Tag T` are refused across a removal, and a live bundle holding them blocks the migration.
