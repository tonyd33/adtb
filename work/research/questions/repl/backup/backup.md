---
title: How a backup of the single file is taken
pick: some
options:
  base-plus-log:
    title: A fuzzy page copy of the file plus the log from the copy's start position
    links:
      - {rel: prior-art, to: prior/repl-pg-backup}
  snapshot-copy:
    title: A read-only transaction writes a consistent compacted copy at one snapshot
    links:
      - {rel: prior-art, to: prior/repl-sqlite-family}
  incremental:
    title: Only pages changed since a previous backup, found by page LSN or log summaries
    links:
      - {rel: prior-art, to: prior/repl-pg-backup}
  replica:
    title: A replica is the backup; its applied log position names the state
---
`layout/files/single-file` makes file-level incremental backup
useless: one file always changes. The redo log with full-page images
and per-page LSNs is what makes an online copy recoverable.

## repl/backup/base-plus-log

Postgres `pg_basebackup`: pages copied while writers run; replay from
the start position repairs torn and inconsistent pages through
full-page images. The copy needs the log up to its end position.

```
backup to "s3://b/app" -- base at 0/1A00, consistent from 0/1C80
```

## repl/backup/snapshot-copy

SQLite `VACUUM INTO` and the backup API's snapshot. Needs no log, and
the copy drops dead versions. A long read pins vacuum, scoped to the
heaps it copies (`mvcc/long-reader/ctor-scoped`), which here is every
heap.

## repl/backup/incremental

Pages whose LSN exceeds the previous backup's start, found by a scan
(`buffer/faults/stale-page/lsn-check` already keeps LSNs on pages), or
from block summaries kept by the log writer as Postgres 17 does.
Restore combines the chain.

## repl/backup/replica

A physical replica plus an archived log is a continuous backup; it
does not protect against a bad write, which it replicates.
