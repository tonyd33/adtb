---
title: What the engine compresses
pick: any
raised-by: [engine/own-pager]
options:
  per-value:
    title: Each large field compressed on its own
    links:
      - {rel: prior-art, to: prior/layout-pg-page}
  per-page:
    title: Whole pages compressed on write to variable-size blocks
    links:
      - {rel: prior-art, to: prior/layout-block-grid}
  dictionary:
    title: A per-heap dictionary replaces repeated values with codes
---
Compression layers stack; none is required.

## layout/compress/per-value

Postgres TOAST compresses a field (pglz or lz4) before moving it out of
line; the value stays decompressible without its page.

## layout/compress/per-page

WiredTiger compresses a page before the block manager writes it, so
on-disk blocks vary in size in 4 KB units and in-memory pages differ
from disk ones. Torn-write handling then covers variable block sizes.

## layout/compress/dictionary

DuckDB dictionary and FSST per column segment. In a per-constructor
heap a `Text` field with few distinct values (country codes) becomes a
`u16` code and the row may become fixed width.
https://duckdb.org/docs/2.0/internals/storage.html
