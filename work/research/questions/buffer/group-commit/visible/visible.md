---
title: A pipelined commit is visible to readers before it is durable
pick: any
raised-by: [buffer/group-commit/pipelined]
options:
  durable-snapshot:
    title: New snapshots see only commits at or below the flushed LSN
  live:
    title: Live with reads of commits that a crash can undo
    excludes: [buffer/group-commit/visible/durable-snapshot]
---
Transaction n+1 ran on top of n before n was durable. A reader that
starts in between and sees n's write, then acts on it externally, has
observed state a crash removes.

## buffer/group-commit/visible/durable-snapshot

Snapshot acquisition reads `flushed_lsn`, not `committed_lsn`; the
writer's own next transaction may run ahead. n+1 depends on n, so if n
is lost so is n+1 (log order).

```
reader begin: snapshot = flushed_lsn
writer:       applies at committed_lsn >= flushed_lsn
```

## buffer/group-commit/visible/live

A crash can retract what a reader saw (unless `engine/isolation`
promises only serializability, which allows it; strict serializability
does not).
