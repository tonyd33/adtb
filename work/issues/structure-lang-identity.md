---
created-at: 2026-11-07
status: open
kind: feature
priority: 16
summary: "about 450 sites key catalog objects by path strings, vector position or name; Schema is open through DerefMut; kernel terms have no validator"
---

# Feature: Catalog identity by id, not spelling or position, in adtb-lang

## Request

## Proposal

## Open questions

## 2026-11-07

Audit: work/briefs/audit-structure-r1-lang.md (local). `Source::ids` keyed by path strings with 7 rewrite arms (evolve/ids.rs `rename_field_ids` rewrites index ids without the constructor); `CatalogIds` by position and name; 114 `ctor_index` sites; `Schema` `DerefMut` to all-pub defs; `u32::MAX` sentinels; kernel terms unvalidated; revert's inverses mirror operators by hand; `Program.checks` aligned with `schema.checks` by position. Subsumes compile-time-identity.

Exit: an `ObjectId` on each definition with `FieldRef`/`CtorRef`; `Schema` closed; a kernel term validator; counts in the report at 0.

## 2026-11-11, design and rulings

Design report: work/briefs/design-lang-identity-r1-report.md (local), on main 34053889. Reproduced live: p1/p1b (renaming a field of one constructor gives a unique and a default over another constructor's same-named field a new id), p2/p3 (renaming a reference field or its constructor gives the implied index a new id), p4 (reverting `drop type` loses value states), p5 (a revert after two renames and a reused name renames the wrong object), p6 (the revert dependency gate compares path strings, so a rename in between hides a dependency). Program: report §4, steps 1 to 10 (5 split into 5a, 5b, 5c).

Rulings (lead):
- Q1: an object's id is carried between operators by a key of resolved ids (a named object by kind, parent id and name; a clause object by relation id, kind, and field and constructor ids). A `Source` keeps declaration text.
- Q2: a typed id per kind in adtb-lang, stored as the same u64, replacing the engine's `Cat*Id` and permission's newtypes; rename keeps an id, drop retires it, none is reused.
- Q3: `CtorRef(TypeId, CtorIdx)` and `FieldRef { def, Header | Ctor, pos }`, minted only by `TypeDef`; `CatalogIds` keyed by them; `Kind::Field` keeps names.
- Q4: `Schema` closes: ids and states are builder inputs, temps are built through the builder, role views are a `Scope`, `RelRef::{Declared, Ahead}`, fields private; `annotate` and `DerefMut` go.
- Q5: the revert dependency gate first becomes a set intersection over `Source::resolved` ids; then each inverse is an operator value by id computed at apply, held by a `Source`-level law search over every operator kind.
- Q6: the kernel validator checks kernel.md §3 typing, sortedness, `Case` arms, `CtorSet` form and binding, §7 normal form as a separate pass; it runs at the exits of check, norm and lowering under debug, the testing feature, the sim and the fuzzer, and once at registration in release; `Terms::add` and `nodes` private.
- Q7: `Program` keys its terms by a typed check or constraint index, read through `program.check(idx)`.
- Compatibility (maintainer, 2026-11-11): no adtb database outside the repository's tests needs to keep opening across these steps. Steps 3 and 5 follow the spec and need no rewrite of stored ids; a fixture holding an old id is regenerated.
