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
