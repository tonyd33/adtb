---
created-at: 2026-10-26
status: open
kind: feature
priority: 768
summary: "no test drives the catalog's refusal of a migration whose new constructor cannot store its smallest row"
tags:
  - quality
  - engine
  - test
---

# Feature: test the refusal of a migration whose constructor's smallest row exceeds the row limit

## Report

`oversized` (`crates/adtb/src/catalog/mod.rs:274-293`, called from `replay`) refuses a migration that declares a constructor whose smallest row does not fit a page. `heap::fits` is tested in `crates/adtb-sim/tests/heap.rs:461-488`, but nothing exercises the catalog refusal, its message, or its restriction to rewritten relations.

Fix: add an api or catalog test that migrates in a record of about 300 `Decimal` fields (fixed layout) and one of about 4000 `Text` fields (slotted layout), and asserts a refusal naming the constructor relation with both byte counts; and one where an existing relation untouched by the migration is not reported.
