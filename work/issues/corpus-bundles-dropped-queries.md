---
created-at: 2026-10-26
status: open
kind: bug
priority: 825
summary: "the spec corpus silently drops a registered query whose parameter types do not resolve, so its explain reports no query"
tags:
  - quality
  - lang
  - test
---

# A corpus registered query with an unresolved parameter type is dropped, and its explain says it registers no query

## Report

`bundles` (`crates/adtb-lang/tests/corpus/mod.rs:446-455`) silently drops a registered query whose parameter types do not resolve. A corpus explain of that query then fails with "registers no query" instead of naming the unresolved type.

Fix: return an error for such a query, as `given_types` does.
