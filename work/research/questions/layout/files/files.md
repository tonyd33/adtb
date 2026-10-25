---
title: How heaps and indexes map to files
pick: one
raised-by: [engine/own-pager]
options:
  per-heap-file:
    title: One file per constructor heap and per index
    links:
      - {rel: prior-art, to: prior/layout-pg-page}
  single-file:
    title: One database file holding every heap and index as page sets
    links:
      - {rel: prior-art, to: prior/layout-sqlite}
  block-grid:
    title: One file of uniform copy-on-write blocks referenced with checksums
    links:
      - {rel: prior-art, to: prior/layout-block-grid}
---
Each constructor is its own heap (settled), so a sum with many
constructors has many heaps, most of them small. Adding a constructor
is a new empty heap; removing one drops a heap.

## layout/files/per-heap-file

Postgres: a relfilenode per relation, split into 1 GB segments, with
side forks (free space map, visibility map) as further files
(unverified segment size). Adding a constructor creates a file; drop is
`unlink`, reclaiming space at once.

```
base/<db>/<heap-id>        identities.EmailPass main fork
base/<db>/<heap-id>_fsm    its free space map
base/<db>/<index-id>       identity_pk union index
```

## layout/files/single-file

SQLite, DuckDB: one file, pages numbered from 0, a header page naming
the catalog root. A heap is a set of pages the catalog points at
(a page list or a B-tree on tids). One fsync target per checkpoint;
a whole database is one file to copy.
https://www.sqlite.org/fileformat2.html

```
page 0      file header: magic, format version, page size, catalog root, free list head
page 1..    catalog, heap pages of every constructor, index pages, free pages
```

## layout/files/block-grid

TigerBeetle: an elastic array of 512 KiB blocks; every reference is
`(u64 address, u128 checksum)`; blocks are never overwritten in place;
a superblock in 4 copies names the roots. WiredTiger is the same shape
with 4 KB allocation units and address cookies.
https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/data_file.md
