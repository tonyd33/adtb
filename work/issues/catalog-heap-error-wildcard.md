---
created-at: 2026-11-11
status: open
kind: bug
priority: 470
summary: "From<catalog::Error> for write::Error sends a catalog heap error other than a page error to Internal through a wildcard, where the write path's own From<heap::Error> maps Changed to a serialization failure"
tags:
  - quality
  - engine
  - class-copied-rules
---

# Catalog heap errors other than a page error reach the client as Internal

## Report

`From<catalog::Error> for write::Error` (crates/adtb/src/write/mod.rs:255-267) lifts only `Heap(heap::Error::Page)`; `Heap(Changed)`, `Heap(TooLarge)` and `Heap(NoDirectory)` fall to `other => Broken::CatalogError(other)`, a `Rejected Internal` reply. The write path's own `From<heap::Error>` (mod.rs:194) maps the same errors otherwise: `Changed` to a serialization failure, `TooLarge` to `Unsupported`, `NoDirectory` to a panic naming its invariant. A second catalog mapping, `catalog_error` in crates/adtb/src/api/registered/mod.rs:731, sends every catalog error but a refusal, `Page` included, to `Internal`.

Reachability is unverified: `TooLarge` looks unreachable for the catalog's fixed layouts; `Changed` depends on whether two catalog writers can delete the same row (candidates catalog/kept.rs:205, catalog/mod.rs:1282). The wildcard hides it either way.

Direction: route a catalog `Heap(e)` through `From<heap::Error>`, or panic naming why a catalog heap cannot meet it; give registered's `catalog_error` the same lifting or state why its callers cannot meet damage. No wildcard in either.

Defect class: Wildcards and copied rules.
