---
title: Where page checksums live and how wide they are
pick: one
raised-by: [engine/own-pager]
options:
  in-page-16:
    title: A 16-bit checksum in the page header mixed with the block number
    links:
      - {rel: prior-art, to: prior/layout-pg-page}
  in-page-32:
    title: A 32-bit CRC-32C in the page header
  in-parent:
    title: A 128-bit checksum held in every reference to the page
    links:
      - {rel: prior-art, to: prior/layout-block-grid}
  none:
    title: No page checksums
---
What corruption the engine can detect on read: bit rot, torn writes,
misdirected writes (right bytes, wrong place), lost writes (old bytes
still there).

## layout/checksum/in-page-16

Postgres: FNV-1a variant over the page as 32 parallel 32-bit sums,
block number XORed in, reduced to `(sum % 65535) + 1`. Catches
misdirected writes; a lost write leaves a valid old page.
https://github.com/postgres/postgres/blob/master/src/include/storage/checksum_impl.h

## layout/checksum/in-page-32

CRC-32C has hardware support (SSE4.2, ARMv8 CRC); Postgres uses it for
WAL records. Same blind spot for lost writes.
https://www.postgresql.org/docs/current/wal-reliability.html

## layout/checksum/in-parent

TigerBeetle: `(u64 address, u128 checksum)` in the referrer, so a
page read from the wrong place or an old version of the right place
fails the check. Needs every page to have one referrer whose copy is
rewritten when the page changes.
https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/data_file.md

## layout/checksum/none

SQLite core: integrity left to the file system (reserved bytes per
page let an extension add one). https://www.sqlite.org/fileformat2.html
