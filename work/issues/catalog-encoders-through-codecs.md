---
created-at: 2026-11-15
status: open
kind: bug
priority: 120
summary: "catalog rows are written by constructor and field name beside the codecs that read them"
tags:
  - engine
  - class-identity-by-spelling
---

# Catalog rows are written by constructor and field name

## Report

Left by structure-lang-identity step 10 (encoders were ruled out of the step). Catalog rows are read through `catalog::codec` by fixed constructor id, but written by constructor and field name in crates/adtb/src/catalog/rows.rs `Builder::push`, catalog/oplog.rs operator rows, api/bundle.rs, catalog/grants.rs `log_row`, api/server/tables.rs, api/server/initialize.rs and api/command.rs `set_setting`. Each panics on a missing name, so a reorder or rename of catalog.adt cannot change a result silently; it is still identity by spelling, and each relation's shape is stated twice.

Fix direction: an `Encode` beside `Decode` in `catalog::codec`, so one codec per relation reads and writes.

## 2026-11-16 count and split

Count (receipt-catalog-encoders-r1): `engine_names` 36 (writers 24: rows.rs 15, bundle.rs 7, oplog.rs 1, values.rs 1; readers or user input 12, of which schema/engine.rs 5 are the `EngineCtors` mint). The codec registry has 12 relations; rows.rs writes 16 more with no codec; `engine_rel`/`engine_ctor` lack ids for migration_ops, migration_clauses and the constructors of those 16; `codec::Sum` handles scalar payloads one level deep; oplog rows feed the migration version hash. Ruled split: A (`Encode` and the 12 coded relations, grants, tables, initialize, bundle, set_setting, the migrations row, dump and adhoc readers, the codec's own layout copies, the pattern widened), B (16 schema-description relations as full codecs), C1 (nested-sum encode), C2 (oplog operator arms, version hash unchanged). The mint in schema/engine.rs stays. sys:: writers by position are out of this ticket.
