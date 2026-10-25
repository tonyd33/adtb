---
title: The old primary keeps committing after a replica is promoted
pick: any
raised-by: [repl/failover/manual, repl/failover/external-lease]
options:
  epoch:
    title: Every log record carries a primary epoch; a node seeing a higher epoch stops and rewinds
  fence:
    title: Promotion first cuts the old primary off from its storage or network
  live:
    title: Live with two divergent histories to reconcile by hand
    excludes: [repl/failover/split-brain/epoch, repl/failover/split-brain/fence]
---
Without consensus nothing stops the old primary from acknowledging
writes no replica will ever see.

## repl/failover/split-brain/epoch

Postgres timelines: promotion starts a new timeline with a history
file. The old primary's commits past the fork point are not on the
new history; rejoining needs a rewind to the fork (`pg_rewind`,
unverified). Clients holding a commit position from the old epoch get
"position not on this history".

```
position = (epoch 4, lsn 0/1A2B)
```

## repl/failover/split-brain/fence

STONITH: the coordinator revokes the old primary's volume or address
before promotion completes.

## repl/failover/split-brain/live

An operator reconciles.
