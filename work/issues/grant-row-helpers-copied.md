---
created-at: 2026-10-29
status: in-progress
kind: bug
priority: 897
summary: "catalog/grants.rs copies rows.rs's tag and boolean and api/bundle.rs's catalog_relation, and adtb-simkit holds a fourth copy"
branch: quality-fixes30
tags:
  - quality
  - engine
---

# catalog/grants.rs copies catalog row helpers

## Report

`crates/adtb/src/catalog/grants.rs:118-133` (`tag`, `boolean`) copy the `tag` and `boolean` methods of the row builder in `crates/adtb/src/catalog/rows.rs:462-478` line for line, as free functions. `grants.rs:109` `catalog_relation` copies `crates/adtb/src/api/bundle.rs:518`, and `crates/adtb-simkit/src/grants.rs:10-30` holds a fourth `catalog_relation` and `tag`. `Shape::log_row` (`grants.rs:74`) builds a row by field name, which the rows.rs builder already does with a check that every field is given.

Fix: one `pub(crate)` set of catalog value helpers in `catalog` (relation by name, nullary tag, Bool), used by rows.rs, grants.rs and api/bundle.rs, and exported for adtb-simkit. Build `log_row` through the rows.rs builder.
