---
created-at: 2026-10-31
status: open
kind: bug
priority: 402
summary: Kind analyses compile with wildcards and catalog rows decode by hand and by position
---

# Compile time identity

## Report

Wildcard arms over `Kind` and operators compile silently when a new form is added, and catalog rows decode by hand, some by constructor position. `plan-binder-wildcards` fixed the planner alone and two more tickets followed.

Deny wildcards over `Kind` and operators in analyses, by a scoped lint. Generate catalog row codecs from `catalog.adt` with adtb-codegen, keyed by catalog constructor id. Add a test that reorders `catalog.adt` constructors and reads an old file the same.

Dissolves bundle-reads-kind-wildcards, translate-kind-wildcards, verify-ctors-by-position, bool-ctor-by-position, bundle-row-decode-lenient, settings-by-ctor-name, initialize-reads-roles-by-hand.

## 2026-11-02: first slice

`fix: deny wildcard arms over Kind in reader and translation analyses` names every form in reads.rs and in `generic`, `compared`, `carries`. A new `Kind` form fails to compile at those sites, shown by mutant. It fixed a false `BundleStale` through `try`, and a wrong answer: a pinned selection over a set kept the pin's constructor positions, so after a removal `(from r in ts select r).B` returned the `C` row and `[B{..},C{..}].C` panicked. Remaining wildcards are on remaining-kind-wildcards. Catalog codecs stay open here.
