---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 825
summary: "the spec corpus silently drops a registered query whose parameter types do not resolve, so its explain reports no query"
closed: 2026-10-27
tags:
  - quality
  - lang
  - test
---

# A corpus registered query with an unresolved parameter type is dropped, and its explain says it registers no query

## Report

`bundles` (`crates/adtb-lang/tests/corpus/mod.rs:446-455`) silently drops a registered query whose parameter types do not resolve. A corpus explain of that query then fails with "registers no query" instead of naming the unresolved type.

Fix: return an error for such a query, as `given_types` does.

## 2026-10-27: done

The corpus `bundles` returns an error for each registered query whose parameter type does not resolve, and the corpus check reports it beside the block's other errors.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the bundles track.
