---
created-at: 2026-10-26
status: open
kind: bug
priority: 766
summary: "open flattens key_schema's and index_keys' typed errors into Corrupt with Debug text, so an internal error reads as corruption"
tags:
  - quality
  - engine
---

# An internal error building an index key at open is reported as corruption with nested Debug text

## Report

At open, `key_schema` and `index_keys` now return the typed catalog `Error`, and `crates/adtb/src/catalog/mod.rs:699-700` and `:702-703` flatten it with `Error::Corrupt(format!("{e:?}"))`. A broken invariant comes out as `Corrupt("Internal(\"derived element 0 of index 3 has a normal program\")")`: an internal failure reported as corruption, with Debug text inside a message.

Fix: propagate `key_schema`'s error with `?`, as `build` does at mod.rs:1053, and for `index_keys` map only `Refused` to `Corrupt`, passing `Internal` and `Corrupt` through.
