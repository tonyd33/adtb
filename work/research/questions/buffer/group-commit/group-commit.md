---
title: How concurrent commits share a log force
pick: one
raised-by: [engine/own-pager]
options:
  leader:
    title: The first committer flushes for everyone queued behind it
    links:
      - {rel: prior-art, to: prior/buffer-pg}
  delay:
    title: The leader waits a short interval for more commits before flushing
    links:
      - {rel: prior-art, to: prior/buffer-pg}
  pipelined:
    title: The writer starts the next transaction while the previous force is in flight
    links:
      - {rel: gains, to: engine/single-writer}
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
  per-commit:
    title: Every commit does its own force
---
One `fdatasync` on NVMe is tens of microseconds to milliseconds
(unverified range); a commit rate above the inverse needs commits to
share a flush.

## buffer/group-commit/leader

Postgres since 9.3: a backend reaching `XLogFlush` takes the lock and
flushes up to the latest inserted LSN; followers wake when the flushed
LSN passes theirs.

```
shared: insert_lsn, flushed_lsn
commit(t): lsn = append(commit record)
           while flushed_lsn < lsn: become leader or wait
```

## buffer/group-commit/delay

Postgres `commit_delay` (default 0) when at least `commit_siblings`
(default 5) transactions are active. Trades latency for batch size.
https://www.postgresql.org/docs/current/runtime-config-wal.html

## buffer/group-commit/pipelined

With one writer the engine applies transaction n+1 to the pool while
the force for n is outstanding, and acknowledges each when its LSN is
durable. TigerBeetle batches requests into prepares and pipelines them.

## buffer/group-commit/per-commit

Throughput capped at one commit per flush latency.
