---
created-at: 2026-10-29
status: open
kind: bug
priority: 897
summary: "initialize reads catalog::roles with its own decoding beside Catalog::committed_roles, and Catalog::roles is pub with callers only in its own file"
tags:
  - quality
  - engine
---

# initialize is a fourth reader of catalog::roles

## Report

role-readers-tripled named three readers and left this one: `initialize` (`crates/adtb/src/api/server/initialize.rs:17-38`) finds the `name` position and matches rows of `catalog::roles` through `committed_rows` to ask whether `owner` exists and whether the table is empty. `Catalog::committed_roles` (`crates/adtb/src/catalog/grants.rs:174`) answers both, and the open path calls it for its own purpose (`crates/adtb/src/api/mod.rs:573`). `initialize` passes over a row of another shape where `committed_roles` reports it as corrupt.

`Catalog::roles` (`crates/adtb/src/catalog/grants.rs:147`) is `pub` and its only callers are in that file, at `:180` and `:285`.

Fix: have `initialize` read `committed_roles`, and make `Catalog::roles` private.
