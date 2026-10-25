---
title: What a replica receives from the primary
pick: one
options:
  physical:
    title: The redo log itself, page records and full-page images
    links:
      - {rel: prior-art, to: prior/repl-pg-physical}
      - {rel: prior-art, to: prior/repl-sqlite-family}
  change-set:
    title: Committed change sets, inserted and deleted rows per constructor relation
    links:
      - {rel: prior-art, to: prior/repl-pg-logical}
      - {rel: gains, to: write/primitive/delta}
  command:
    title: Transaction inputs, re-executed by every replica in one order
    links:
      - {rel: prior-art, to: prior/repl-calvin}
      - {rel: prior-art, to: prior/repl-vsr}
    assessment: {grade: costly, why: "under SSI the log must carry a serial order, and every transaction including ad hoc ones must execute deterministically"}
---
Decides whether a replica is a byte copy of the single file or a
second engine applying writes, and so whether pages can be repaired
from it, whether it may run another engine version, and what a
replica must re-derive (indexes, constraint checks).

## repl/unit/physical

Postgres streaming replication, dqlite (WAL frames through Raft),
Litestream and LiteFS (WAL pages as LTX files). The replica runs redo
recovery forever. Pages, TIDs and constructor ids are identical to the
primary's, so a page that fails its checksum can be refetched from a
replica and both sides can compare page checksums.

```
replica r1 { source: primary, unit: physical }
```

## repl/unit/change-set

Each commit ships the change set the transaction applied, the value
`write/primitive/delta` already denotes: inserted and deleted
relations per constructor, with the commit position. A retag is a
delete from one constructor and an insert into another in the same
set. The replica rebuilds its own heaps and union indexes, trusts the
primary's constraint checks, and may differ in page layout. Postgres
logical replication is the precedent, minus its gap: typed schema
operators are log entries too, so DDL replicates.

```
changeset @0/1A2B (xid 812) {
  identities.EmailPass  - { id: 9, user: 3, email: "a@x", hash: h }
  identities.Google     + { id: 9, user: 3, gid: "x" }
}
```

## repl/unit/command

Calvin and TigerBeetle replicate inputs: a registered transaction name
and its arguments, executed on every replica in a fixed order.
rqlite does this with SQL statements and must special-case
`RANDOM()` and time.

```
log 0/1A2B: retag_to_google(user: 3, gid: "x")
```
