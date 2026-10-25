---
title: How running transactions meet a schema change
pick: some
raised-by: [evolution/smo]
options:
  exclusive-ddl:
    title: The operator waits for and blocks every transaction on the affected sums
  online-states:
    title: The operator passes through intermediate states, each compatible with the last
    links:
      - {rel: prior-art, to: prior/engine-f1-schema}
      - {rel: prior-art, to: prior/engine-fdb-record-layer}
      - {rel: gains, to: evolution/old-readers/readers-first}
  pinned-version:
    title: Each transaction runs against the schema version of its snapshot, translated across versions
    links:
      - {rel: prior-art, to: prior/prism-bidel}
---
Registered queries are compiled against a schema version; a split
changes which constructor relations exist. Online states and pinned
versions stack: staged states roll a change out, pinned versions keep
serving queries checked against an older version.

## engine/exclusive-ddl

Postgres `ACCESS EXCLUSIVE`, SQLite's schema cookie invalidating
prepared statements (https://www.sqlite.org/fileformat2.html). Simple;
a long reader stalls the change and everything queued behind it.

## engine/online-states

F1: adding constructor `Apple` goes declared (readers can type it, no
writes), then writable; adding an index goes write-only, backfilled,
then readable, as the Record Layer's index states.

```
0011 add constructor Identity.Apple { sub: Text }   -- state: declared
0012 enable identities.Apple                       -- state: writable
```

## engine/pinned-version

InVerDa (Herrmann et al. SIGMOD 2017, unverified): several schema
versions coexist; a transaction pinned to v11 reads and writes through
the operators' forward and backward mappings. Relies on the accepted
operators having inverses.
