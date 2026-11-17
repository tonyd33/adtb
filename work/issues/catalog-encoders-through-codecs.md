---
created-at: 2026-11-15
status: resolved
closed: 2026-11-16
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

## 2026-11-16, part A landed

Landed as rknzqrry e9f2552f (2 rounds), full check 0 (3853 tests). The 12 coded relations are written through `Codec::encode` by field handle (grants log, roles, grants, audit, initialize, bundles, settings, the migrations row); dump and adhoc look relations up by id. Encoding byte-identical to main (files, dumps, cross restore, checked by review). `engine_names` pattern widened to any `push` receiver; 39 to 28 (rows.rs 17, part B). Resolved constructor-field-layout-copied. Known limit: the distinct-value test gives sum fields position modulo constructor count, so a swap of two same-typed sum fields can pass. Next: part B.

## 2026-11-16, part B landed

Landed as nwwztxvy fb316b1a (2 rounds), full check 0 (3853 tests). 14 schema-description relations are full codecs in `Coded` (catalog/described.rs), 21 fixed `engine_ctor` constants (lookups into the existing table, ids unchanged), writers through `Codec::encode`/`Codec::tag`. Byte identity with main checked by the author and repeated by the review. `engine_names` rows.rs 17 to 3. Left: migration_ops and migration_clauses (sums nested deeper than `codec::Sum`), with `values::relation`/`values::row`. Next: C1 (nested-sum encode), C2 (operator arms, version hash unchanged).

## 2026-11-16, part C landed, resolved

Landed as mtnykxoz 80030471 (2 rounds), full check 0 (3854 tests). The codec encodes nested sums by fixed constructor id at every level (`Kind`, `Payload`, `Nested`, `Table` in catalog/codec.rs; payloads in catalog/operators.rs); migration_ops and migration_clauses are codecs in `Coded`; `Builder::push`, `values::row` and `values::relation` removed. The version hash is unchanged: catalog/hash_corpus_tests.rs pins 37 hashes taken from a main build over a corpus reaching every constructor of the operator types, derived from the declared types. Writers by name: 0. `engine_names` 28 to 9: the 5 sites of schema/engine.rs are the `EngineCtors` mint and stay; task.rs, readers.rs, evolve/cx.rs and types.rs look up names the user wrote. The sys:: rows written by position are sys-rows-written-by-position.
