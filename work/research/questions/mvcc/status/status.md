---
title: How a reader learns whether a version's writer committed
pick: one
raised-by: [mvcc/xid-clog]
options:
  clog-lookup:
    title: Every visibility check consults the commit log, versions are never touched by readers
  hint-bits:
    title: The first reader after commit caches the outcome in the version header
    links:
      - {rel: prior-art, to: prior/mvcc-pg-heap}
      - {rel: prior-art, to: prior/mvcc-undo-engines}
---
The commit log is 2 bits per xid; a lookup is an SLRU page access and
must follow the in-progress check.

## mvcc/status/clog-lookup

No header bits for commit status. Every tuple read pays a clog probe
unless the xid precedes the oldest running xid and freezing has
stamped it. Pages are written only by writers and vacuum.

## mvcc/status/hint-bits

Postgres `HEAP_XMIN_COMMITTED`, `HEAP_XMIN_INVALID`,
`HEAP_XMAX_COMMITTED`, `HEAP_XMAX_INVALID`. A committed hint may be set
only once the commit record is flushed past the page LSN. Oracle's
delayed block cleanout is the same pattern: the next reader copies the
commit SCN into the block's ITL and generates redo.
