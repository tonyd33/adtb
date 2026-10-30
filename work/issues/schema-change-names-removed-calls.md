---
created-at: 2026-10-29
status: open
kind: bug
priority: 850
summary: "schema-change.md still gives catalog.stale and catalog.source_at as the recheck calls after both methods were deleted, and says stale needs an enabled constructor where the code takes a deprecated one too"
tags:
  - quality
  - design
---

# schema-change.md states recheck calls that no longer exist

## Report

af41cfc3 deleted `Catalog::stale` and `Catalog::source_at`, leaving the free functions `catalog::stale(source, now, reader)` and `catalog::source_at(source, v)` (`crates/adtb/src/catalog/readers.rs:720`, `:753`). `docs/design/schema-change.md:279-280` still lists `catalog.source_at(v)` and `catalog.stale(&reader)` as the calls a registering subsystem relies on, the postconditions at `:295-301` describe those methods, and `:378-380` defines the free functions by reference to them, so the contract now defines them by calls that do not exist.

The same postcondition says `stale` returns a constructor "enabled at the catalog's version". `stale` takes any stage but `Declared` (`readers.rs:733`, `:739`), so a `Deprecated` constructor absent at the reader's version is named too. The code is the behaviour to keep, since a deprecated constructor still holds rows.

Fix: state the two free functions in the call list and postconditions, drop `:378-380`, and write "not `Declared`" in place of "enabled".
