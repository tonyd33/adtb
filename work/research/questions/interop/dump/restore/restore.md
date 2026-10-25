---
title: How a restore checks constraints
pick: one
raised-by: [interop/dump/logical]
options:
  one-set:
    title: The restore is one change set over every relation, checked at its end
    links:
      - {rel: variant-of, to: interop/load/unit/one-set}
  offline-validate:
    title: Rows are written unchecked into a database not yet open, then every constraint query runs once
---
`pg_dump` orders sections (definitions, data, then indexes and
constraints) because SQL checks per statement
(https://www.postgresql.org/docs/current/app-pgdump.html). Multiple
assignment makes order irrelevant: references and cycles are fine
inside one change set.

## interop/dump/restore/one-set

The restore is `interop/load` with every relation in one
`transact`. Needs the bulk check path.

## interop/dump/restore/offline-validate

The new file is not a database until validation passes: no reader
ever sees a state, so no statement boundary is crossed unchecked.
Every constraint is a query returning violations
(`constraint/form/empty`); a nonempty result fails the restore with
the report.
