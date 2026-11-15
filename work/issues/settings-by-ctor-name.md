---
created-at: 2026-10-27
status: resolved
closed: 2026-11-15
kind: bug
priority: 895
summary: "file settings are read by matching constructor name spellings and field position, and a row of any other shape is ignored silently"
tags:
  - quality
  - engine
---

# File settings are read by constructor name spelling and field position

## Report

`Settings::set` (`crates/adtb/src/api/settings.rs:63-75`) dispatches on the strings `"DefaultLease"`, `"LeaseGrace"` and `"RetryLimit"` and reads `fields.get(1)`. Its `_ => {}` arm silently drops a setting whose constructor is renamed or whose value field moves, and a negative `RetryLimit` becomes 0 through `unwrap_or(0)`.

Fix: resolve the settings sum's constructor ids once from the schema, and refuse an unexpected shape or a negative limit as corrupt.

## Resolved

Resolved by structure-lang-identity step 10a, nsrtnoty dc62bc7b (2 rounds, full check 0, 3800 tests). `catalog::codec` decodes `catalog::settings` and `catalog::roles` rows through one codec per relation built from the `Schema`: constructors by engine fixed id (`ids::engine_ctor`, relations by `engine_rel`), fields by compile-time handles checked against catalog.adt both ways at build. A row of another shape is refused at open as `Rot::CatalogRow` with its whole reason; `Settings::set` names every constructor; `Catalog::roles` is private and `initialize` reads `committed_roles`. A test stores each setting under the shipped layout and reads it under a schema with every sum reversed. Negative counts and durations still read as 0, ticket negative-setting-accepted.
