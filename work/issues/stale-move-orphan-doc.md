---
created-at: 2026-10-29
status: open
kind: bug
priority: 897
summary: "schema_at's doc comment was left above the moved stale, and Catalog::stale and Catalog::source_at are left with no caller"
tags:
  - quality
  - engine
---

# schema_at's doc comment sits above stale

## Report

When checked-registration moved the body of `Catalog::stale` into a free `stale`, it left `schema_at`'s doc line ("Returns the schema of the version `v` of `source`'s log.") at `crates/adtb/src/catalog/readers.rs:713`, above `stale`'s own doc. `stale` now carries two docs and `schema_at` (`readers.rs:762`) none. This is the fourth time a moved function has left its doc this way, after `revert-reorder-orphan-doc`, `reference-upsert-doc-misplaced` and `catalog-orphan-doc-comment`.

The same change left `Catalog::stale` (`readers.rs:401`) and `Catalog::source_at` (`readers.rs:395`) with no caller in the workspace. They are `pub`, so nothing warns.

Fix: move the line back above `schema_at`, delete the two methods (or keep one caller), and add a `house-check` rule that flags a doc comment with two summary sentences that both start "Returns" above one item.
