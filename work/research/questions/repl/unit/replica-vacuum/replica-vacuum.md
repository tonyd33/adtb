---
title: Replayed vacuum removes versions a replica snapshot still reads
pick: any
raised-by: [repl/unit/physical]
options:
  feedback:
    title: Replicas report their oldest snapshot, per constructor heap, into the primary's watermark
    links:
      - {rel: gains, to: mvcc/long-reader/ctor-scoped}
      - {rel: prior-art, to: prior/repl-pg-physical}
  cancel:
    title: A conflicting replica reader is cancelled after a bounded replay delay
  delay-replay:
    title: Replay pauses on a conflicting record until the reader ends
  live:
    title: Live with replica readers failing whenever vacuum runs
    excludes: [repl/unit/replica-vacuum/feedback, repl/unit/replica-vacuum/cancel, repl/unit/replica-vacuum/delay-replay]
---
Per-sum vacuum on the primary logs the removal of dead versions; a
replica replaying it destroys versions its own readers' snapshots can
see. Postgres has exactly this conflict on hot standby.

## repl/unit/replica-vacuum/feedback

Postgres `hot_standby_feedback`, made per heap: a registered query on
a replica names the constructor relations it reads, so its snapshot
holds back vacuum only of those heaps on the primary.

```
replica r1 feedback { identities.Google: xmin 812 }
```

## repl/unit/replica-vacuum/cancel

`max_standby_streaming_delay`: replay waits up to a bound, then the
reader gets a typed "snapshot removed by replication" error. A
read-only safe-snapshot transaction can then abort, which
`engine/retry/safe-snapshot` promised it never does.

## repl/unit/replica-vacuum/delay-replay

The replica falls behind for as long as its longest reader runs.

## repl/unit/replica-vacuum/live

Readers fail without a bound.
