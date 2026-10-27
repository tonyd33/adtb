---
created-at: 2026-10-27
status: open
kind: bug
priority: 863
summary: "check_feed resolves names in one module while queries and transactions beside it resolve over the bundle's visible modules"
tags: [quality, lang]
---

# A feed over an imported relation is refused while a query over it is admitted

## Report

`check_feed` at `crates/adtb-lang/src/check/mod.rs:990` uses `Visible::module(module)`. This range added `check_query_in` and `check_txn_in` (`check/mod.rs:970`, `check/write.rs:1632`), which take a `Visible` for the bundle. The corpus calls `check_feed` with one module (`crates/adtb-lang/tests/corpus/mod.rs:417`). A feed over an imported module's relation is refused while a query over the same relation checks.

`check_feed` and `crates/adtb-lang/src/evolve/grant.rs:78-87` each carry a copy of the "is not a relation" lookup.

Fix: add `check_feed_in(schema, vis, f)` beside the other two, and share the relation lookup.
