---
title: Failover after an asynchronous commit loses acknowledged transactions
pick: any
raised-by: [repl/commit-ack/local]
options:
  stated:
    title: The spec states the loss window as commits past the promoted replica's applied position
  wait:
    title: A client may wait on a commit's position reaching a replica after the fact
  live:
    title: Live with silent loss of acknowledged commits on failover
    excludes: [repl/commit-ack/rpo/stated, repl/commit-ack/rpo/wait]
---
LiteFS: unreplicated writes on a failed primary may be lost when a new
primary takes over.

## repl/commit-ack/rpo/stated

Promotion reports the position it starts from; transactions after it
on the old primary are gone, and the old primary's tail is a separate
history (see `repl/failover/split-brain`).

## repl/commit-ack/rpo/wait

```
let c = commit t durability local
await replicated c.position on quorum
```

## repl/commit-ack/rpo/live

No statement.
