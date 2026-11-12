---
created-at: 2026-11-11
status: open
kind: bug
priority: 350
summary: "a schema built mid-migration gives an object with no log id its declaration-order id, which can equal another object's log id, and sites break the tie differently"
tags:
  - class-identity-by-spelling
---

# Mid-migration ids shared between objects

## Report

## 2026-11-11

`Source::annotate` sets log ids over declaration-order ids and leaves the declaration-order id on an object the log has no id for yet. That id can equal another object's log id: probes during structure-lang-identity step 1 found 383 shared entries, all in `annotate` calls where some object had no log id. Two production paths see such schemas: evolve's operator modules call `self.schema()` mid-operator, before `Source::apply_recording` runs `rename_ids` and `reconcile_ids` (the grant carry path too), and the engine routes mid-step rows by these ids (`catalog::layout::row` from `moved::moved`, `Catalog::mapped`, `Catalog::apply`, `Catalog::migrate`). Step 1 kept every site's old tiebreak (reverse lookups return all holders least first; former `find` sites take the least, former map-building sites the greatest), so two sites can resolve the same id to different objects. Giving unlogged objects ids from a disjoint range broke split and merge migrations (`catalog::ids::stored`: "a constructor id fits 32 bits"), since rows are routed by those ids.

Direction: structure-lang-identity step 3 (objects keyed by resolved ids) removes declaration-order ids; route mid-step rows by the resolved key, not a numeric id; then make reverse lookups single-valued and assert it. A test: a split migration whose new constructor's declaration-order id equals an existing object's log id, reading every site that resolves it.

Defect class: Identity by spelling: a string or position stands in for the identity the type system could hold.
