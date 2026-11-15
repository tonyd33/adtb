---
created-at: 2026-10-31
status: resolved
closed: 2026-11-15
kind: bug
priority: 402
summary: Kind analyses compile with wildcards and catalog rows decode by hand and by position
tags:
  - engine
  - class-identity-by-spelling
---

# Compile time identity

## Report

Wildcard arms over `Kind` and operators compile silently when a new form is added, and catalog rows decode by hand, some by constructor position. `plan-binder-wildcards` fixed the planner alone and two more tickets followed.

Deny wildcards over `Kind` and operators in analyses, by a scoped lint. Generate catalog row codecs from `catalog.adt` with adtb-codegen, keyed by catalog constructor id. Add a test that reorders `catalog.adt` constructors and reads an old file the same.

Dissolves bundle-reads-kind-wildcards, translate-kind-wildcards, verify-ctors-by-position, bool-ctor-by-position, bundle-row-decode-lenient, settings-by-ctor-name, initialize-reads-roles-by-hand.

## 2026-11-02: first slice

`fix: deny wildcard arms over Kind in reader and translation analyses` names every form in reads.rs and in `generic`, `compared`, `carries`. A new `Kind` form fails to compile at those sites, shown by mutant. It fixed a false `BundleStale` through `try`, and a wrong answer: a pinned selection over a set kept the pin's constructor positions, so after a removal `(from r in ts select r).B` returned the `C` row and `[B{..},C{..}].C` panicked. Remaining wildcards are on remaining-kind-wildcards. Catalog codecs stay open here.

## Resolved

Catalog codecs landed through structure-lang-identity steps 10a, 10b-1 and 10b-2 (nqkvnwok 97b3ccc9, 2 rounds, full check 0, 3830 tests). Every catalog relation the engine reads outside the roots heap is decoded by a `catalog::codec` codec built from the schema, constructors and nested sums by engine fixed id, fields by compile-time handles, payload types checked both ways at build; `committed_rows` is private to catalog; structure-check rows `catalog_row_fields` and `catalog_row_ctors` reached 0 and were removed. Reversed-catalog tests read a sample row per constructor under the shipped and the reversed catalog. A table-driven codec replaced codegen. Kind wildcards stay on remaining-kind-wildcards; verify kept rows and roots rows are step 11.
