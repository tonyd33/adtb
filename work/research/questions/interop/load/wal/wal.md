---
title: What a bulk load writes to the redo log
pick: one
raised-by: [interop/load/source, interop/load/command, interop/load/append-api]
options:
  page-images:
    title: Each newly filled heap and index page is logged once as a full image
    links:
      - {rel: variant-of, to: layout/torn/fpw}
  unlogged-new:
    title: Pages new to this transaction are forced to the file at commit and not logged
  row-redo:
    title: Each inserted row is a redo record, as any insert
---
Redo-only logging with full-page images (`engine/redo-mvcc`,
`layout/torn/fpw`) means a load is written twice. Replicas that
replay the log (`repl/`) need the rows in it.

## interop/load/wal/page-images

The writer fills pages in private memory and logs each finished page
whole; no per-row records, and a page image is what a torn-write
repair needs anyway.

## interop/load/wal/unlogged-new

Postgres `wal_level = minimal` skips WAL for a relation created or
truncated in the same transaction and syncs it at commit (unverified
detail). A log-shipping replica never sees those rows, so this is
unavailable with physical replication.

## interop/load/wal/row-redo

No special path; the log is several times the data.
