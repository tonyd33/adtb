---
created-at: 2026-10-26
status: open
kind: feature
priority: 894
summary: "forward relations are RelId(u32::MAX - i), unattached checks RelId(u32::MAX) and unresolved element types TypeId(u32::MAX), and three passes stay aligned only by skipping the same sentinel"
tags:
  - quality
---

# Feature: typed placeholders for forward relations, unattached checks and unresolved element types

## Report

Placeholders are still ids picked from the top of the id space:

- `crates/adtb-lang/src/schema.rs:250` and `:386`: a relation a later operator adds is `RelId(u32::MAX - i)`, held in `Ty::Key`. `Schema::relation` (schema.rs:304) panics on it, and every consumer of a `Key r` type must call `ahead_relation` first (schema.rs:594, :2220, `crates/adtb/src/catalog/layout.rs:129`).
- schema.rs:1055: a check whose relation is not yet resolved is `RelId(u32::MAX)`. `crates/adtb-lang/src/check/mod.rs:594`, `crates/adtb-lang/src/analysis/mod.rs:57` and `crates/adtb/src/catalog/validate.rs:100` each skip it, and validate.rs zips `qualified(schema)` with `normal.checks` by position (validate.rs:60), so the names line up only while all three skip exactly the same checks.
- schema.rs:1008 with :1298, :1518, :2016, `crates/adtb-lang/src/ids.rs:435`, `crates/adtb-lang/src/resolve.rs:236` and `crates/adtb-lang/src/evolve/ids.rs:127`: a relation whose element type is unresolved has `elem_def: TypeId(u32::MAX)`.

Fix: give `Ty::Key` a target that is either a `RelId` or an ahead index (`enum KeyTarget { Rel(RelId), Ahead(u32) }`), make `CheckDef::relation` an `Option<RelId>`, and `RelationDef::elem_def` an `Option<TypeId>` during build only, or keep unresolved relations out of `relations`. Have validate.rs pair each check with its name from the normal program rather than by position.
