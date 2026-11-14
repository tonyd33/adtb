---
created-at: 2026-11-14
status: open
kind: feature
priority: 360
summary: "Schema::declared panics on a mid-migration schema, held only by a runtime check; a committed schema type carrying declared would make the precondition a type"
tags:
  - structure
  - class-failure-flattened
---

# Feature: a committed schema type that alone carries `declared`

## Report

`Schema::declared` (crates/adtb-lang/src/schema/mod.rs) reads a key's relation and panics on a schema with relations a later operator adds; since the ahead-declared track it panics whenever `ahead` is non-empty, whatever key it is given. Three of its callers were judged committed-only and proved reachable mid-migration (step 6b rounds 2 and 3, fill-print-of-ahead-key-panics). The precondition is held by tracing each caller's schema to where it is made, and by that panic.

Direction: a `Committed` schema type that only a schema with no ahead relations can give, carrying `declared`, held by every caller's schema source without an `expect` at each. Sources measured: engine `Catalog.schema` (catalog/mod.rs:170), the write path's `Arc<Schema>` fields (write/mod.rs:470, 601, 658), api `Arc<Schema>` fields (api/mod.rs:349, 762, 782; api/registered/mod.rs:174, 187, 301, 493; api/server/call.rs:118; api/text.rs:57; api/maintenance.rs:53, 106), the reference database's schema, `Signature.schema` (signature.rs:332), translate `Map.from`, codegen, adtb-file functions taking `&Schema`, sessions made by `with_temps`; producers `build`, `build_with`/`relabelled`, `Source::schema`, `with_temps`. 20+ fields across five crates: more than one track. First step: `Catalog.schema: Committed` with `Deref<Target = Schema>`.

Defect class: Damage or failure gives a panic, hang or flattened error.
