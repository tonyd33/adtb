---
title: Byte order of stored integers
pick: one
raised-by: [substrate/new-engine]
options:
  little:
    title: Little-endian everywhere in pages and rows
    links:
      - {rel: prior-art, to: prior/layout-union-encoding}
  big:
    title: Big-endian everywhere
    links:
      - {rel: prior-art, to: prior/layout-sqlite}
  native:
    title: The host's byte order
---
Row and page bytes only. Index keys compared by `memcmp` need an
order-preserving encoding (big-endian, sign bit flipped, tag first for
sums) whatever the row format, as the FoundationDB tuple layer does
(https://github.com/apple/foundationdb/blob/main/design/tuple.md).

## layout/endian/little

Cap'n Proto, TigerBeetle (unverified), and every common CPU; no swap on
load, files portable.

## layout/endian/big

SQLite: files portable; a byte swap on each load on x86 and ARM.

## layout/endian/native

Postgres: no swap, but a data directory is not portable across byte
orders (unverified).
