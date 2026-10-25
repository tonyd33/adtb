---
title: Copy-on-write moves pages but TIDs name page numbers
pick: any
raised-by: [layout/torn/cow]
options:
  page-table:
    title: A mapping table from logical page id to physical location
  live:
    title: Live with rewriting union index entries when a heap page moves
    excludes: [layout/torn/cow-tid/page-table]
links:
  - {rel: gains, to: layout/checksum/no-parent/heap-directory}
---
Union index entries hold `(ctor, tid)`. If a heap page's location
changes on every write, the TID must name something stable.

## layout/torn/cow-tid/page-table

Bw-tree mapping table (Levandoski et al., ICDE 2013, unverified): a
TID's page part is logical; the table, itself copy-on-write and
checkpointed, gives the current block. Same structure as a heap
directory with in-parent checksums.

## layout/torn/cow-tid/live

Every page write rewrites the entries pointing into it: unbounded.
