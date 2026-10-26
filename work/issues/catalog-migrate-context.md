---
created-at: 2026-10-26
status: open
kind: feature
priority: 892
summary: "Catalog::migrate threads pager, transactions, transaction and snapshot through helpers with too_many_arguments, links a removed Catalog::extend, and admits over heap ids build never assigns"
tags:
  - quality
---

# Feature: one migration context in the catalog

## Report

In crates/adtb/src/catalog/mod.rs:

- `replay` (mod.rs:723) and `emptied` (mod.rs:795) carry `#[expect(clippy::too_many_arguments)]` whose reasons name the arguments instead of what would remove the expectation. `holds`, `build` and `write_rows` take the same `pager`, `txns` and `snapshot`, and `Built` already bundles `x` and `snapshot` for two of them.
- The doc comment of `admits` (mod.rs:688) links `Catalog::extend`, which this range removed.
- `admits` (mod.rs:690-720) predicts a heap id for each constructor relation: the existing heap's id where one exists, else the next free id. `build` (mod.rs:855) gives every constructor of a rewritten relation a new heap, so the plan catalog `admits` checks constraints over names heap ids the migration never assigns. The constraint refusals do not depend on the ids today, but nothing states that.
- The migration's source is checked three times on the open path: `next.schema()` in `migrate`, `source.check()` in `admits`, and `catalog.source().check()` in `Engine::open` (crates/adtb/src/api/mod.rs).

Fix: one `Migrating` context holding pager, transactions, `x` and snapshot, passed to `replay`, `emptied`, `holds`, `build` and `write_rows`, which removes both expectations; fix the link; have `admits` take its heap ids from the same assignment `build` makes, or state in its doc that the ids are placeholders; check the source once and keep the checked program on the catalog.
