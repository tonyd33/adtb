---
created-at: 2026-10-29
status: open
kind: bug
priority: 898
summary: "CatalogIds gained a reverse lookup for relations only, and constructor, field, type and set ids are still found by inline scans in six files"
tags:
  - quality
  - lang
  - engine
---

# Reverse id lookups stay inline for everything but relations

## Report

`CatalogIds::relation` (`crates/adtb-lang/src/ids.rs:105`) replaced the relation scans, and the same scan for the other id kinds remains:

- constructors: `crates/adtb-lang/src/evolve/grant_text.rs:142`, `crates/adtb-lang/src/translate/mod.rs:312`, `crates/adtb-lang/src/permission/grants.rs:80`, `crates/adtb/src/catalog/readers.rs:868`, `crates/adtb/src/catalog/ids.rs:90`;
- fields: `crates/adtb-lang/src/evolve/grant_text.rs:147`, `crates/adtb-lang/src/translate/mod.rs:313`, `crates/adtb-lang/src/permission/grants.rs:90`, `crates/adtb/src/catalog/pending.rs:88`;
- types: `crates/adtb/src/catalog/ids.rs:79`;
- sets: `crates/adtb-lang/src/evolve/grant_text.rs:125`.

`catalog::ids::relation` (`crates/adtb/src/catalog/ids.rs:72`) is now a one-line wrapper. lang-id-follow-ups covers the id types, not these lookups.

Fix: add `ctor`, `field`, `type_def` and `set` reverse lookups beside `CatalogIds::relation` and call them at these sites.
