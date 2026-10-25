---
title: How the per-sum vacuum pass holds the dead entries it removes
pick: one
raised-by: [union-index/vacuum/batched]
options:
  sorted-array:
    title: Sorted array of constructor and tid, binary searched per entry
  radix:
    title: Radix tree keyed by constructor and block, offsets as a bitmap
    links:
      - {rel: prior-art, to: prior/btree-art-olc}
  per-heap-bitmap:
    title: One dead-offset bitmap per heap page, indexed by constructor then block
---
Batched vacuum scans every constructor heap of the sum, collects dead
`(ctor, tid)`, then makes one pass over each union index asking, per
entry, "is this dead?". The set's size bounds how many dead rows one
pass handles before the indexes are scanned again.

## btree/dead-set/sorted-array

Postgres before 17: flat array of 6-byte TIDs, capped at 1 GB (about
179M TIDs) (unverified figure). With constructors, 10 bytes per pair
under a `u32` id; lookup `log n`.

## btree/dead-set/radix

Postgres 17 TidStore: adaptive radix tree from block to an offset
bitmap, up to 20x smaller than the array (unverified figure). Keyed
`ctor ++ block`, a sum's heaps are disjoint subtrees.
https://pganalyze.com/blog/5mins-postgres-17-faster-vacuum-adaptive-radix-trees

```
key: ctor_be32 | block_be32   ->   bitmap of dead offsets (<= 291 at 8 KiB)
```

## btree/dead-set/per-heap-bitmap

A dense bitmap per heap, `blocks * max_offsets` bits; constant-time
lookup, size proportional to heap size not dead count.
