---
title: Whether record and sum types are structural or nominal
pick: one
options:
  nominal:
    title: Every record and sum type is equal only to itself by declared name
  structural:
    title: Types are equal when their fields and constructors are equal
    links:
      - {rel: prior-art, to: prior/third-manifesto}
      - {rel: prior-art, to: prior/lang-machiavelli}
  nominal-sums:
    title: Sums are nominal and records are structural
---
`{ email: Email }` declared twice under two names: one type or two. Query
results build record shapes nobody declared; schema evolution renames
things by name.

## model/nominal

```
type A = { x: Int }
type B = { x: Int }
-- A and B are distinct; a value of A is not a B
```

Haskell, Rust. Soufflé record types are declared by name (unverified
whether two equal declarations unify, https://souffle-lang.github.io/types).

## model/structural

A type is its heading. Third Manifesto tuple and relation types are
generated from headings (RM Prescriptions 6, 7); Machiavelli and NRC
records are structural; DuckDB `STRUCT(a INT)` is structural
(https://duckdb.org/docs/sql/data_types/struct). Sums are structural
too: polymorphic variants, OCaml `[ `A | `B ]`.

## model/nominal-sums

Declared sums are nominal so exhaustiveness and evolution have a name to
attach to; records are structural so projections and joins produce types
without declarations. Elm (records structural, custom types nominal,
https://guide.elm-lang.org/types/), OCaml objects vs variants.
