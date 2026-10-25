---
title: A client that wrote on the primary then reads on a replica
pick: any
raised-by: [repl/standby-reads/safe-snapshot]
options:
  bookmark:
    title: Commit returns a log position; a replica read given it waits until a safe snapshot covers it
    links:
      - {rel: prior-art, to: prior/repl-sqlite-family}
  sticky:
    title: The client library keeps a session on one node and its last position
  apply-ack:
    title: The commit waits until named replicas have applied it
    needs: [repl/commit-ack/applied]
  live:
    title: Live with a client not seeing its own write on a replica
    excludes: [repl/session/bookmark, repl/session/sticky, repl/session/apply-ack]
---
Serializable alone allows a later read to precede an earlier write in
the serial order, so read-your-writes across nodes is a separate
promise.

## repl/session/bookmark

Cloudflare D1 sessions carry a bookmark and give sequential
consistency; libSQL gives read-your-writes only to the writing
replica.

```
let c = commit t                 -- c.position = 0/1A2B
on replica r1 after c.position { from g in identities.Google ... }
```

## repl/session/sticky

The bookmark is hidden in the client: every read in the session
carries the highest position it has seen.

## repl/session/apply-ack

Postgres `remote_apply`: the write is visible on the replica before
the commit returns.

## repl/session/live

No promise across nodes.
