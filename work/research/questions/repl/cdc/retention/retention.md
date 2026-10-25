---
title: A stalled consumer pins the log forever
pick: any
raised-by: [repl/cdc/change-stream, repl/unit/change-set]
options:
  bounded:
    title: A consumer position older than a retention bound is invalidated
  resnapshot:
    title: An invalidated consumer restarts from an exported snapshot and the position after it
    links:
      - {rel: prior-art, to: prior/repl-pg-logical}
  consumer-ack:
    title: Consumers acknowledge a position stored in the database, giving at-least-once with dedup by position
  live:
    title: Live with unbounded log growth behind a dead consumer
    excludes: [repl/cdc/retention/bounded, repl/cdc/retention/resnapshot, repl/cdc/retention/consumer-ack]
---
A Postgres replication slot retains WAL and catalog rows with no
consumer attached; its position is persisted only at checkpoint, so a
crash resends changes. Checkpoints here flush the oldest pages so the
log can be truncated, which a slot would block.

## repl/cdc/retention/bounded

```
subscribe identities_feed ... retain 7d
```

## repl/cdc/retention/resnapshot

The subscription records the snapshot the stream starts after, as
Postgres slot creation exports one.

## repl/cdc/retention/consumer-ack

Debezium resumes from its recorded LSN; storing the acked position in
the database makes resumption exact.

## repl/cdc/retention/live

Disk fills.
