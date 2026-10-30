---
created-at: 2026-10-30
status: open
kind: bug
priority: 500
summary: "verify writes Finding and JobKind constructors as literal positions and stores them in the file by position, so a constructor declared earlier renumbers kept findings"
tags:
  - quality
  - engine
---

# verify writes Finding and JobKind constructors as literal positions, and keeps them in the file by position

## Report

`crates/adtb/src/api/verify.rs:20` has `VERIFY_JOB = 3` and `fields` at `:58` maps each `Found` to the literals 0 to 6, by the declaration order of `JobKind` and `Finding` in `crates/adtb-lang/src/catalog.adt:331` and `:351`. `KeptRow.ctor` (`crates/adtb/src/catalog/kept.rs:36`) then stores that position in the database file, and `val` at `verify.rs:43` reads it back as the constructor.

A new `Finding` constructor declared anywhere but last renumbers the rows of every existing file without a `ROW_FORMAT` change: a kept `Order` reads as another kind, or as a row with the wrong fields, and nothing refuses it. A wrong literal compiles. Constructor ids are the identity that is never reused, and `catalog::values::tag` already resolves a constructor by name.

The `bool-ctor-by-position` ticket covers file scans only.

Fix direction: resolve the positions by name from the schema at open, and store the catalog constructor id in a kept row, mapping it back on read. Add a test that a file's kept finding keeps its kind after a later constructor is declared before it, or refuse by format if that is the ruling.
