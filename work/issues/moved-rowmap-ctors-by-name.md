---
created-at: 2026-11-13
status: open
kind: feature
priority: 700
summary: "catalog/moved.rs still matches constructors and fields between the old and new schema by name at about 15 sites, which the lang identity program assigned to step 3 and no later step accounts for"
tags:
  - engine
  - quality
---

# Feature: Migration row conversion by id in catalog/moved.rs

## Report

Step 2 of the lang identity program counted 24 constructor-by-name sites that are not name resolution and assigned "catalog/moved.rs x10" to step 3. Step 3 changed crates/adtb/src/catalog/moved.rs by one line, and no later step's record accounts for these sites.

Live at main:

- `ctor_named` at moved.rs:548, :816, :866 and :926.
- `ctor_index(&into)` at :657 and :705.
- `ctor_index(&name)` at :667, :717, :832 and :878.
- `ctor_index(&inner.name)` at :770.
- Fields matched by name in `copies` (:515, :523), `lift` (:266), `by_name` (:281), `carried` (:292) and `held_fills` (:485).

Some of these resolve `RowMap`'s text operands (`into`, `discriminator`, `field`), which may be name resolution by design. Two are cross-schema identity by spelling:

- `route` (:547) takes the constructor a routing returns in `routing.schema`, prints its name, and finds the product in schema b by that name.
- `copies` finds the source field by name, where `KeyAt::new` (:327) matches by field id.

Found by reading; no wrong conversion reproduced.

Fix direction: have `RowMap` carry constructor and field ids (or `CtorRef`/`FieldRef` of the schema each side names), and match a-side and b-side fields by id as `KeyAt` does. Record the outcome against the program's step-2 count.
