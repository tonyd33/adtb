---
title: What a heap page header carries beyond its free-space bounds
pick: any
raised-by: [engine/own-pager]
options:
  lsn:
    title: The LSN of the last log record applied to the page
    needs-one-of: [engine/redo-mvcc, engine/aries]
  checksum:
    title: A checksum of the page computed at write
  owner:
    title: The owning heap or index id and the page's own number
  version:
    title: The page's format version
  prune-hint:
    title: The oldest deleting transaction on the page, so vacuum skips clean pages
    needs: [engine/in-heap]
---
Every page has `lower`, `upper` (or a slot count) and flags. These are
the fields that stack on top. Postgres's 24-byte header holds lsn,
checksum, flags, lower, upper, special, size and version, prune xid.
https://www.postgresql.org/docs/current/storage-page-layout.html

A header taking all of them:

```
heap page (8 KiB)
  0   u64 lsn
  8   u32 checksum        crc32c, field zeroed while computing
  12  u16 flags           all-visible, has-free-slot, ...
  14  u16 format          per-page format version of the owner's schema
  16  u32 owner           heap id = constructor id from the catalog
  20  u32 page_no         own page number, catches misdirected writes
  24  u64 prune_xid       oldest xmax on page, 0 if none
  32  u16 lower           end of slot array
  34  u16 upper           start of tuple area
  36  ...                 slot array or fixed slots
```

## layout/header/lsn

Recovery redoes a record only when its LSN exceeds the page's; the
buffer pool flushes a page only after the log is durable to its LSN.
Not needed under copy-on-write or logical redo.

## layout/header/checksum

Detects corruption and torn writes on read. Width and placement in
`layout/checksum`.

## layout/header/owner

A union index entry `(key, ctor, tid)` routes by `ctor`; a page that
records its owner lets a probe check it landed in the heap the entry
names, and lets salvage rebuild the catalog from pages. Postgres instead
mixes the block number into the checksum.
https://github.com/postgres/postgres/blob/master/src/include/storage/checksum_impl.h

## layout/header/version

`pd_pagesize_version` in Postgres. Lets one heap hold pages of several
row formats, each page uniform: see `layout/heap-format`.

## layout/header/prune-hint

Postgres `pd_prune_xid`: oldest unpruned xmax, zero if none.
