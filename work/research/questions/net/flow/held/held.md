---
title: A stalled result stream pins its snapshot
pick: any
raised-by: [net/flow/pull, net/flow/credit, net/flow/socket]
options:
  idle-limit:
    title: A stream idle past a limit is cancelled with a typed error
  spool:
    title: The server materializes the rest and releases the snapshot
  live:
    title: Live with slow readers holding back vacuum
    excludes: [net/flow/held/idle-limit, net/flow/held/spool]
---
Versions visible to an open snapshot cannot be vacuumed
(`union-index/vacuum/batched`); a client that stops reading holds
them.

## net/flow/held/idle-limit

`Error StreamIdle { after: 30s }`; FoundationDB's 5 s limit plays the
same role.

## net/flow/held/spool

Spill to a temp file; costs disk.

## net/flow/held/live

Vacuum waits on the slowest reader.
