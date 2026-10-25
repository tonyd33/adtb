---
title: External relations are outside constraints and isolation
pick: any
raised-by: [interop/foreign/file-scan, interop/foreign/attach]
options:
  no-constraints:
    title: Constraints, keys and references may not mention an external relation
  pinned-file:
    title: A transaction reads each file once at a recorded version and checksum
  live:
    title: Live with external reads that are neither serializable nor constrained
    excludes: [interop/foreign/integrity/no-constraints, interop/foreign/integrity/pinned-file]
---
A file can change between two reads of one transaction; a remote
table can violate any constraint stated over it. `postgres_fdw` does
not import check constraints because they may evaluate differently
remotely (https://www.postgresql.org/docs/current/postgres-fdw.html).

## interop/foreign/integrity/no-constraints

External relations have a distinct kind: `Key logs` is a type error,
and a constraint query ranging over one is refused at schema time like
an unbounded one (`constraint/admit/bounded`).

## interop/foreign/integrity/pinned-file

Parquet and Arrow files are immutable once written; object stores
give an ETag or version. The first read records it; a later read in
the same transaction that sees another version aborts.

## interop/foreign/integrity/live

A query result mixing stored and external rows is not a snapshot.
