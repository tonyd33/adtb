---
title: Restoring to a point in the past
pick: some
raised-by: [repl/backup/base-plus-log]
options:
  log-archive:
    title: Continuous log archive; restore replays a base to a position, time or named point
    links:
      - {rel: prior-art, to: prior/repl-pg-backup}
  object-store:
    title: Log segments shipped to object storage with periodic snapshots
    links:
      - {rel: prior-art, to: prior/repl-sqlite-family}
  epoch-on-restore:
    title: A restore starts a new history epoch so old and new logs never mix
    links:
      - {rel: same-as, to: repl/failover/split-brain/epoch}
  op-log-point:
    title: A schema operator is a named restore point
    links:
      - {rel: gains, to: surface/source/op-log}
---
Postgres: recovery targets by time, xid, LSN or name; finishing
starts a new timeline. Litestream picks the newest snapshot not past
the target and applies LTX files in order. D1 Time Travel restores
any minute of the last 30 days in place and returns a bookmark to
undo the restore.

## repl/pitr/log-archive

```
restore "s3://b/app" to before op 0012
restore "s3://b/app" to time "2026-10-24T13:02Z"
```

## repl/pitr/object-store

The embedded shape: no server, the library ships log segments.

## repl/pitr/epoch-on-restore

Postgres timelines and history files.

## repl/pitr/op-log-point

Restoring to just before a migration is the common case; the linear
operator log names every version, so `to before op 0012` is exact.
