---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 797
summary: "a bounded generic function, a log whose migration differs and holds an unresolved operator, and removing a constructor that still has rows each panic"
closed: 2026-10-27
tags:
  - engine
  - lang
---

# Bug: panics found while writing the manual

## Report

- A schema function with a bounded type parameter, `fn least [a: Ord] (...)`, panics at `crates/adtb-lang/src/canon.rs:103`.
- Opening a database whose log has a migration N that differs from the one presented, where the logged one holds an operator with no resolved form, panics at `crates/adtb/src/catalog/oplog.rs:40` ("logged migration 7 resolves"). It should refuse with the "differs from the log's" refusal.
- `remove constructor` together with `redefine fn`, on a deprecated constructor that still has rows, panics at `crates/adtb/src/catalog/change.rs:170`. It should refuse, naming the rows.

## 2026-10-27: done

- `canon::decl_names` records a function's type parameter bounds as prelude names. Test: `catalog::tests::a_function_with_a_bounded_type_parameter_is_hashed`.
- `catalog::try_version_hash` is the fallible version hash; `api::same_hash` uses it, so a shipped migration with no resolved form diverges. Test: `oplog::a_shipped_migration_with_an_operator_of_no_resolved_form_diverges`.
- `Catalog::replay` stops at the first refused data step. Test: `oplog::removing_a_constructor_that_holds_rows_with_a_redefined_function_is_refused`.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the q799 track.
