---
created-at: 2026-11-13
status: open
kind: bug
priority: 600
summary: "kernel.md states the schema's id contract through CatalogIds fields that are private, and schema-change.md still describes the ahead relations and id reconciling as they were before steps 6a-2 and 6b"
tags:
  - design
  - quality
  - class-none
---

# The design contracts for schema ids and ahead relations are stale

## Report

docs/design/kernel.md §6 (lines 482 to 507) states where a consumer finds catalog ids: "each relation's key index in `CatalogIds::keys` and its uniques in `CatalogIds::indexes`", a view's row type, constructor and fields "in `view_types`, `view_ctors` and `view_fields`", and an expression unique's id "from `Schema::ids.indexes`". Since step 1 these are private fields of `CatalogIds` (crates/adtb-lang/src/ids/mod.rs:102 to 132). Consumers read them through `key_id`, `index_id`, `index_ids`, `view_type_id`, `view_ctor_id` and `view_field_id`, which the contract does not name.

docs/design/schema-change.md:599 to 604 says ids "are reconciled after each step". It also says `Schema::ahead_relation(r)` names a `Key r` whose `r` a later operator adds, and that `Schema::relation(r)` panics on it.

- Since step 6a-2, named objects take ids per operator from the parsed declarations, and only clause objects are reconciled over a built source.
- Since step 6b, a key type holds `RelRef::Ahead(AheadId)`. `ahead_relation` takes an `AheadId`, `Schema::relation_of` refuses an ahead relation, and `Schema::declared` panics on one.

Found by reading.

Fix direction: restate both contracts in terms of the accessor methods, `RelRef`, `relation_of` and `declared` (with its precondition), and the 6a-2 rule for named and clause ids.

Defect class: class-none
