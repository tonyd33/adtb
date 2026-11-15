---
created-at: 2026-10-30
status: resolved
closed: 2026-11-15
kind: bug
priority: 500
summary: "verify writes Finding and JobKind constructors as literal positions and stores them in the file by position, so a constructor declared earlier renumbers kept findings"
tags:
  - quality
  - engine
  - class-identity-by-spelling
---

# verify writes Finding and JobKind constructors as literal positions, and keeps them in the file by position

## Report

`crates/adtb/src/api/verify.rs:20` has `VERIFY_JOB = 3` and `fields` at `:58` maps each `Found` to the literals 0 to 6, by the declaration order of `JobKind` and `Finding` in `crates/adtb-lang/src/catalog.adt:331` and `:351`. `KeptRow.ctor` (`crates/adtb/src/catalog/kept.rs:36`) then stores that position in the database file, and `val` at `verify.rs:43` reads it back as the constructor.

A new `Finding` constructor declared anywhere but last renumbers the rows of every existing file without a `ROW_FORMAT` change: a kept `Order` reads as another kind, or as a row with the wrong fields, and nothing refuses it. A wrong literal compiles. Constructor ids are the identity that is never reused, and `catalog::values::tag` already resolves a constructor by name.

The `bool-ctor-by-position` ticket covers file scans only.

Fix direction: resolve the positions by name from the schema at open, and store the catalog constructor id in a kept row, mapping it back on read. Add a test that a file's kept finding keeps its kind after a later constructor is declared before it, or refuse by format if that is the ruling.

## Resolved

Landed as structure-lang-identity step 9c, nlzuqomn c85d1afb (2 rounds, full check 0, 3790 tests). verify names `JobKind` and `Finding` constructors by engine fixed ids (`ids::engine_ctor`, resolved by name in `TYPES` at compile time, so a missing name or another type's constructor fails to compile; `each_engine_constructor_constant_is_the_engine_constructor_of_its_name` checks them against catalog.adt). `KeptRow.ctor` is `Option<CatCtorId>`, stored as the id with no constructor as 0 (a const assert keeps 0 off every constructor); `ROW_FORMAT` 2, and a file whose kept rows are format 1 refuses at open with "a kept row has format 1; this engine reads format 2". A finding row whose id is not a `Finding` constructor, or that has none, is refused as damage with the whole text pinned. The reorder test cannot be written (the engine's declarations are compiled in); the fixed-id test, a round trip by constructor name and the refusal tests stand in. Filed beside: kept-finding-fields-unchecked.
