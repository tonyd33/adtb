---
title: What a load does with bad rows
pick: one
raised-by: [interop/load/source, interop/load/command, interop/load/append-api]
options:
  abort:
    title: Any bad row aborts the load and returns the violation report
  reject-input:
    title: Rows failing parse or routing go to a rejects relation; constraint violations abort
    links:
      - {rel: prior-art, to: prior/interop-duckdb-io}
      - {rel: prior-art, to: prior/interop-pg-copy}
  reject-violations:
    title: Rows violating constraints are also diverted and the rest committed
    assessment: {grade: costly, why: "removing one row can break FKs and totality of others, so the kept set is a fixpoint"}
---
Bad rows come in two kinds: input that does not parse into the row
type or routes nowhere, and well-typed rows that violate a constraint.
The violation report of a constraint is already a relation
(`constraint/form/empty`).

## interop/load/errors/abort

The load returns the first failing constraint's result relation, with
source file and line attached.

## interop/load/errors/reject-input

```
let (rows, rejects) = read_csv("identities.csv", IdentityRow) with rejects
-- rejects : Set { line: I64, column: Option Text, error: Text, raw: Text }
```

Postgres 17 `ON_ERROR ignore` covers only type input and still fails
on constraints; DuckDB `store_rejects` writes `reject_errors` with
line, column, error type and the original line
(https://duckdb.org/docs/2.0/data/csv/reading_faulty_csv_files.html).

## interop/load/errors/reject-violations

The engine computes the largest subset of the delta whose application
satisfies every constraint. Dropping an `EmailPass` row whose email
collides makes its user violate `at-least-one` identity, and so on.
