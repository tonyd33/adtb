---
title: How concurrent threads synchronize on index pages
pick: one
raised-by: [btree/structure/blink]
options:
  page-locks:
    title: Shared and exclusive page latches, one page at a time, move right on split
    links:
      - {rel: prior-art, to: prior/btree-nbtree}
  olc:
    title: Optimistic lock coupling with a version word per page
    links:
      - {rel: prior-art, to: prior/btree-art-olc}
---
Physical latching inside the tree, separate from transaction
isolation (`engine/cc`). Latches protect page bytes; locks or read sets
protect keys.

## btree/latch/page-locks

Postgres: readers share-latch one page, copy every matching
`(ctor, tid)` of the leaf to local memory, release, then visit heaps;
moving right or up may hold the next latch, moving left or down never.
An inserter couples child and parent latches on ascent only. Readers
write the latch word, so the root's cache line bounces between cores.

## btree/latch/olc

Leis et al. (DaMoN 2016), LeanStore (ICDE 2018), OLFIT (VLDB 2001):

```
page latch word (u64):  version 62 bits | locked 1 | obsolete 1
read:   v = load; if locked spin; ...read...; if load != v restart from root
write:  CAS locked on; modify; store version+1, unlocked
```

Readers never write shared memory. A reader must validate before
following any pointer it read, and a freed page must stay mapped until
no reader can hold it (epochs).
