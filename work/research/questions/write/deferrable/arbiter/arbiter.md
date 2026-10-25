---
title: A deferred key cannot decide an upsert or a retag
pick: any
raised-by: [write/deferrable/unique]
options:
  non-arbiter:
    title: Deferrable keys cannot be upsert targets
    links:
      - {rel: prior-art, to: prior/write-pg-constraints}
  eager-index:
    title: The union index is always checked eagerly; only the error is deferred
  live:
    title: Live with ambiguous upserts
    excludes: [write/deferrable/arbiter/non-arbiter, write/deferrable/arbiter/eager-index]
---
An upsert asks "does a row with this key exist" at the statement. With
the key deferred, two live rows may share it mid-transaction, and the
union index move for retag assumes at most one live entry per key.

## write/deferrable/arbiter/non-arbiter

Postgres: only `NOT DEFERRABLE` constraints and unique indexes arbitrate
`ON CONFLICT`.

## write/deferrable/arbiter/eager-index

Duplicates are recorded in the index and counted; commit fails if any
remain. An upsert on a key with recorded duplicates errors.

## write/deferrable/arbiter/live

The upsert picks one of the matching rows.
