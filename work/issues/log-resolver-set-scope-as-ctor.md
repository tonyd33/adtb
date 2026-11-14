---
created-at: 2026-11-13
status: open
kind: bug
priority: 320
summary: "the log resolver reads a named set scope of add index or unique as a constructor, so a revert whose dependants include one is refused"
tags:
  - class-identity-by-spelling
---

# The log resolver looks a set scope up as a constructor

## Report

`add index (..) in r.SET` or `add unique (..) in r.SET`, scoped by a named set, does not resolve in the log: `Resolver::ctor` looks the set name up as a constructor. Any revert whose dependency check resolves such a later migration is then refused with "the log does not resolve: ... gone by the migration's end". Found writing a law case in structure-lang-identity step 5c-3, which used a constructor scope instead.

Direction: the resolver resolves a scope name to a set or a constructor by what the source declares, keeping its catalog id. Test: a later `add index (n) in ts.R` (R a set) does not stop an unrelated revert.

Defect class: Catalog rows and identity by position or spelling.

## Also alone in a migration

The engine refuses `add index (z) in ts.S` and `add unique ... in r.SET` naming a named set even alone in a migration, with "an object `OP` creates or names is in neither the source before it nor the one after it" (was "an object this operator creates or names is gone by the migration's end" before b5496637): crates/adtb-lang resolved.rs:774 resolves each selector ident as a constructor. Reproduced on main and in structure-lang-identity step 6a-2's review.
