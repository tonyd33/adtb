---
title: How wide a stored transaction id is
pick: one
raised-by: [mvcc/xid-clog, mvcc/csn-map]
options:
  xid32:
    title: 32-bit ids compared modulo 2^32, with an in-memory epoch
    links:
      - {rel: prior-art, to: prior/mvcc-pg-heap}
  xid64:
    title: 64-bit ids in every version, never wrapping
    links:
      - {rel: prior-art, to: prior/mvcc-undo-engines}
---
Ids live in every version header, so width is per-row space.

## mvcc/xid-width/xid32

Postgres: `FullTransactionId` is `epoch << 32 | xid` in memory, 32 bits
on disk; any normal xid has 2^31 older and 2^31 newer. 8 bytes per
version for xmin and xmax.

## mvcc/xid-width/xid64

OrioleDB OXid. 16 bytes per version for xmin and xmax; no freezing.
A 64-bit id per page as base with 32-bit offsets per tuple would halve
that (Postgres Pro, unverified).
