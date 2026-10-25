---
title: How the own pager makes commits durable
pick: one
raised-by: [engine/own-pager]
options:
  redo-mvcc:
    title: Redo-only WAL, old versions stay in heaps so no undo
  aries:
    title: ARIES redo and undo with in-place updates
  shadow:
    title: Copy-on-write pages with an atomic root switch, no log
    links:
      - {rel: prior-art, to: prior/engine-embedded-kv}
  logical-redo:
    title: Log committed row versions only, rebuild indexes on recovery
    links:
      - {rel: prior-art, to: prior/engine-hekaton-calvin}
---
A retag writes two heaps and every union index over the sum; recovery
must never leave the row in both heaps or neither.

## engine/redo-mvcc

Postgres: page-level redo records, full-page images after checkpoint,
aborted versions left in heaps for vacuum. A retag is several records
in one transaction; atomicity comes from commit visibility, not undo.
https://www.postgresql.org/docs/current/wal-intro.html

## engine/aries

Mohan et al., TODS 1992: steal, no-force, undo by compensation log
records. Pairs with an undo-log version store (InnoDB, Oracle).

## engine/shadow

LMDB: new pages for every change, meta page switched last. One writer.

## engine/logical-redo

Hekaton: indexes are never logged. Recovery replays row versions and
rebuilds union indexes, which costs a full build per sum after a crash.
