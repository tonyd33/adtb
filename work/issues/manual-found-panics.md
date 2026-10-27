---
created-at: 2026-10-27
status: open
kind: bug
priority: 797
summary: "a bounded generic function, a log whose migration differs and holds an unresolved operator, and removing a constructor that still has rows each panic"
tags:
  - engine
  - lang
---

# Bug: panics found while writing the manual

## Report

- A schema function with a bounded type parameter, `fn least [a: Ord] (...)`, panics at `crates/adtb-lang/src/canon.rs:103`.
- Opening a database whose log has a migration N that differs from the one presented, where the logged one holds an operator with no resolved form, panics at `crates/adtb/src/catalog/oplog.rs:40` ("logged migration 7 resolves"). It should refuse with the "differs from the log's" refusal.
- `remove constructor` together with `redefine fn`, on a deprecated constructor that still has rows, panics at `crates/adtb/src/catalog/change.rs:170`. It should refuse, naming the rows.
