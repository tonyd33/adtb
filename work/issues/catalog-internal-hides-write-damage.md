---
created-at: 2026-11-11
status: open
kind: bug
priority: 440
summary: "catalog::Broken::KeyCounter and GrantId wrap a write::Error whose damage is not a Page, so it reaches the client as Rejected Internal"
tags:
  - class-failure-flattened
---

# Catalog guarantee errors hide write-path damage

## Report

## 2026-11-11

`catalog::Broken::KeyCounter` (catalog/mod.rs) and `catalog::Broken::GrantId` (catalog/grants.rs) carry a `write::Error` inside a catalog `Internal`. Damage other than a page error inside them (`write::Error::Corrupt`, an orphan entry) is not lifted by structure-transactions step 8's `From<catalog::Error> for write::Error`, which lifts only catalog-level damage, so it reaches `write::Broken::CatalogError` and the client as `Rejected Internal`. Damage is never `Internal` (catalog.md §4 keeps it for checker guarantees).

Direction: return the inner `write::Error` itself where it is damage (or have the `From` look inside these two variants), so `fail` maps and marks it; a test planting damage on the key counter's or grant id's page.

Defect class: Damage or failure gives a panic, hang or flattened error.
