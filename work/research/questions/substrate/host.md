---
title: What the system is built on
pick: one
options:
  pg-compiler:
    title: Schema and query compiler emitting plain Postgres
    links:
      - {rel: prior-art, to: prior/gel}
  pg-extension:
    title: Postgres extension with new types, operators or index methods
  duckdb-extension:
    title: DuckDB extension building on its UNION type
    links:
      - {rel: prior-art, to: prior/duckdb-union}
  new-engine:
    title: New database engine
    links:
      - {rel: prior-art, to: prior/apg}
---
Where the ADT layer lives decides which constraints can be enforced by
the engine and which must be emulated with triggers or a write API.

## substrate/pg-compiler

An ADT schema language and a query language with `match`, compiled to
standard DDL, views, triggers and SQL. Postgres runs unmodified.

```
type Provider = EmailPass { email: Text, hash: Text }
              | Google    { gid: Text }
relation providers : Provider
  key id
  unique (email) in EmailPass
  unique (gid) in Google
```

emits one table per constructor, any key or claim table, the union view,
and the triggers the chosen encoding needs.

## substrate/pg-extension

A Postgres extension (for instance via pgrx) adding what plain DDL cannot
express: a sum type, constructor-aware operators, possibly an index
method. Postgres index access methods index a single relation, so an
index spanning several tables is not obviously possible (unverified).

## substrate/duckdb-extension

Build on DuckDB's native `UNION` column type and extension API. Inherits
a value-level sum for free; relation-level sums would still be encoded.

## substrate/new-engine

A database designed around sums from the start: storage, constraints,
planner and query language. Free to add primitives such as a unique
index over a disjoint union of relations.
