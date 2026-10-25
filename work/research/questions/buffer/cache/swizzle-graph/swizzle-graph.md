---
title: Swizzling needs one owning reference per page but heap pages are named by TIDs
pick: any
raised-by: [buffer/cache/swizzle]
options:
  heap-tree:
    title: Store each constructor heap as a B+tree on tid so every page has one parent
    links:
      - {rel: prior-art, to: prior/buffer-leis}
      - {rel: variant-of, to: layout/checksum/no-parent/heap-directory}
  tid-table:
    title: Swizzle only tree edges and translate heap TIDs through a hash table
  live:
    title: Live with it
    excludes: [buffer/cache/swizzle-graph/heap-tree, buffer/cache/swizzle-graph/tid-table]
---
LeanStore and Umbra unswizzle a child by finding its single parent
(parent pointer in the frame) and rewriting the swip. A union index
entry `(key, ctor, tid)` names a heap page; thousands of entries in
many index pages name the same page, so there is no one swip to
rewrite on eviction. Leis et al. (SIGMOD 2023, sec. 2) give this as
the reason swizzling does not support graph data.

## buffer/cache/swizzle-graph/heap-tree

Umbra's shape: a relation is a B+tree keyed by a monotonically
increasing 8-byte tuple id; index entries hold the tid, a lookup
descends the heap tree (two or three swizzled hops when hot). The
settled `(key, ctor, tid)` entry stays; `tid` becomes a key, not a
page address.

```
heap identities.Google   B+tree on tid (u64)
  inner 64 KiB: [swip child]* separators
  leaf:         rows tid 4096..8191, fixed-width columns PAX
union index entry: (key, ctor_id = Google, tid = 5120)
```
https://www.cidrdb.org/cidr2020/papers/p29-neumann-cidr20.pdf

## buffer/cache/swizzle-graph/tid-table

B-tree pages swizzled; heap pages found through a `(heap, page) ->
frame` table as in `buffer/cache/hash-pool`. Two translation
mechanisms, two eviction paths.

## buffer/cache/swizzle-graph/live

Swizzling is unavailable for heap pages.
