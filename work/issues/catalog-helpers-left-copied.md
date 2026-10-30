---
created-at: 2026-10-29
status: open
kind: bug
priority: 896
summary: "catalog relation lookups and by-name row builders survive beside catalog::values in migration, oplog, initialize and the server tables"
tags:
  - quality
  - engine
---

# Catalog helpers stay copied outside catalog::values

## Report

grant-row-helpers-copied made `crates/adtb/src/catalog/values.rs` the one set of helpers for the four copies that ticket named. These remain:

- Relation by name, `values::relation`: `crates/adtb/src/catalog/migration.rs:688`, `crates/adtb/src/catalog/oplog.rs:94`, `crates/adtb/src/api/server/initialize.rs:17`, `crates/adtb/src/api/server/tables.rs:60`.
- Type of module `catalog` by name, with no helper yet: `crates/adtb/src/api/bundle.rs:287` and `:298`, `crates/adtb/src/api/server/tables.rs:210` and `:261`.
- Row by field name: `oplog::sum` (`crates/adtb/src/catalog/oplog.rs:114`) repeats the length assert and find-by-name of `values::row`, and `Tables::row` (`crates/adtb/src/api/server/tables.rs:128`) does the same over `Val`. `initialize` builds its row by a `match` on field names with a panic arm (`crates/adtb/src/api/server/initialize.rs:41`), the form the ticket removed from `grants.rs` and adtb-simkit.
- `Shape::log_row` gets the `Some` constructor by calling `tag`, documented for nullary constructors, and destructuring with `unreachable!` (`crates/adtb/src/catalog/grants.rs:76`).

Fix: use `values::relation` at the four sites, add a `values` lookup of a catalog type and of a constructor index for the type sites and `Some`, build `sum` and `initialize`'s row over `values::row`'s by-name step.
