---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 799
summary: "evolution.md 4.6 calls a split routing invertible by fixing a field to a nullary constructor, but 4.7 refuses a derived merge whose discriminator names do not match the products"
closed: 2026-10-27
tags:
  - spec
  - evolution
---

# Feature: Spec gap in split and merge invertibility

## Report

evolution.md 4.6 calls a split routing invertible when it fixes a field to a nullary constructor and says the inverse merge is derived. 4.7 refuses a merge whose discriminator's sum does not have exactly the products' names as constructors. A derived inverse built this way names its discriminator after the split's own routing, not the products, so reverting a split is refused. This blocks reverting the campaign app's 0003 (scaling `Realm | Town | Spot` into `Region | Settlement | Site`).

Proposed ruling for the lead: the derived inverse of a split carries the routing's map from discriminator constructors to products, so 4.7's name rule applies only to a merge written by hand, not to a derived inverse.

## 2026-10-27: ruling

The derived inverse of a split carries the split's routing: its map from discriminator constructors to products, from the values of a `Bool` field to products, or, for a split into one product, the identity. The name rule of `evolution.md` section 4.7 applies only to a merge an author writes, never to a derived inverse. State this in 4.6 and 4.7 and make the engine follow it. The schema operator law tests pin it with two ignored tests in `crates/adtb-sim/tests/oplog.rs`: `reverting_a_split_by_a_bool_field_merges_the_rows_back_with_the_field` and `reverting_a_split_into_one_product_merges_the_rows_back`. Remove `split by bool` and `split into one product` from the `UNREVERTED` skip list in `crates/adtb-testkit/tests/differential.rs` once they pass.

## 2026-10-27: done

The derived inverse of a split is a merge applied with `derived` set: its arms' fixed values (`wide: true`, `scale: Realm`) tell the merged constructors apart in place of a discriminator field, so the name rule applies only to an author's merge. Uniques re-scope over the fixed fields; a check over some of the merged constructors becomes a `match` on the one fixed field when there is exactly one. `evolution.md` 4.6 and 4.7 state it. `split by bool` and `split into one product` are off the `UNREVERTED` list; the two oplog tests run (the bool test's expected record was corrected to the printed field order), and two evolve tests cover a discriminator named otherwise (the campaign 0003 shape) and a check on one product.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qlaws track.
