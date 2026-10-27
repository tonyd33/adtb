---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 863
summary: "check_feed resolves names in one module while queries and transactions beside it resolve over the bundle's visible modules"
closed: 2026-10-27
tags:
  - quality
  - lang
---

# A feed over an imported relation is refused while a query over it is admitted

## Report

`check_feed` at `crates/adtb-lang/src/check/mod.rs:990` uses `Visible::module(module)`. This range added `check_query_in` and `check_txn_in` (`check/mod.rs:970`, `check/write.rs:1632`), which take a `Visible` for the bundle. The corpus calls `check_feed` with one module (`crates/adtb-lang/tests/corpus/mod.rs:417`). A feed over an imported module's relation is refused while a query over the same relation checks.

`check_feed` and `crates/adtb-lang/src/evolve/grant.rs:78-87` each carry a copy of the "is not a relation" lookup.

Fix: add `check_feed_in(schema, vis, f)` beside the other two, and share the relation lookup.

## 2026-10-27: done

`check_feed_in(schema, vis, f)` checks a feed over a bundle's visible modules, and `check_feed` calls it with one module. The relation lookup is `resolve::lookup_relation`, used by `check_feed_in`, the log grant resolver and the resolver's relation references. Test: `check::tests::writes::a_feed_resolves_over_the_modules_its_bundle_sees`. Still open: the corpus harness calls `check_feed` with the bundle's one imported module, as it does for its queries, and the API registers no feed yet.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qread track.
