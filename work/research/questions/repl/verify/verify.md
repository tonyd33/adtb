---
title: How a backup or replica is shown to be good
pick: some
raised-by: [repl/backup/base-plus-log, repl/backup/snapshot-copy, repl/backup/incremental, repl/backup/replica]
options:
  manifest:
    title: A manifest of page and segment checksums, checked without restoring
    links:
      - {rel: prior-art, to: prior/repl-pg-backup}
  restore-check:
    title: Restore, replay, then re-check every sum invariant over the result
  rolling-checksum:
    title: A running checksum of the whole database, compared between primary and replica
    needs: [repl/unit/physical]
    links:
      - {rel: prior-art, to: prior/repl-sqlite-family}
---
`pg_verifybackup` checks checksums and that the needed WAL parses,
and states it cannot catch WAL with correct checksums but nonsensical
actions.

## repl/verify/manifest

Fast; catches missing and bit-rotted pages.

## repl/verify/restore-check

The engine knows its invariants: exclusivity (one heap per key),
totality, FKs over constructor sets, union index against heaps. A
restored copy is opened, recovered, and checked.

```
verify restore "s3://b/app" at 0/1C80 checks all
```

## repl/verify/rolling-checksum

LiteFS keeps a rolling checksum of the full database contents in its
replication position. Byte-identical physical replicas make the
comparison meaningful per page.
