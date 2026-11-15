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
