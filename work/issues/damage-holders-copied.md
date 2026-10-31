---
created-at: 2026-10-30
status: in-progress
branch: quality-fixes33
kind: bug
priority: 896
summary: "Database::holders and Catalog::holders each map a damaged page to its object, with the engine-module rule twice and different answers for the roots heap"
tags:
  - quality
  - engine
---

# Damage holders copied

## Report

`crates/adtb/src/write/quarantine.rs:48` and `crates/adtb/src/catalog/mod.rs:843` both walk every heap, history heap, index and history index into `Pager::holders_of`, each with its own copy of the `ENGINE_MODULES` rule, and must stay aligned on what maps to what.

They differ on the roots heap: a damaged page there marks the database at open (`Some(Quarantine::Database)`) and marks nothing once running (`h.walk(None, ...)` over `kept_store`). Nothing states that difference where either copy can see it.

Fix direction: one walk over a description both can build, taking the roots heap's object as a parameter.
