---
created-at: 2026-11-16
status: open
kind: bug
priority: 750
summary: "Codec::field_type and Codec::table find a field's constructor by its field list's names, so a handle shared by several constructors returns the first one's type"
tags:
  - engine
  - quality
  - class-identity-by-spelling
---

# A codec field handle shared by several constructors reads as the first constructor's field

## Report

Found by the quality review of reviewed..main (c93c399e..92dbbc64) on main 92dbbc64.

A `Field` is a `Fields` list (`&'static [&'static str]`) and a position. `Codec::field_type` (crates/adtb/src/catalog/codec.rs:673-685) and `Codec::table` (crates/adtb/src/catalog/codec.rs:687-698) find the constructor by `shape.ctor.fields == f.fields`, which compares the lists' names, and take the first match. `field_type`'s doc says it returns the stored type of field `f`, with the precondition only that a constructor of the codec lists `f`.

In the setting codec, `LIMIT` (crates/adtb/src/catalog/codec.rs:1235) is the field list of four constructors (crates/adtb/src/catalog/codec.rs:1340-1355). catalog.adt declares `StatementLimit { limit: Option Duration }` and `StreamIdleLimit { limit: Duration }`, so `field_type` of the `limit` handle returns `Option Duration` for a caller that means `StreamIdleLimit`. `Row::get` accepts any handle whose list is spelled the same, so a handle of one constructor reads another's field.

Latent: the only callers of `field_type` (crates/adtb/src/catalog/oplog.rs:103-104) and of `table` are on one-constructor relations.

Fix direction: give `Field` and `Sum` the constructor's fixed id where a codec hands them out, and look up by that id, or key `field_type` and `table` by constructor id and field. Comparing slice addresses is no fix: equal constant slices may be merged.
