---
title: How free pages of a shared file are tracked
pick: one
raised-by: [layout/files/single-file, layout/files/block-grid]
options:
  freelist:
    title: Free page numbers listed in trunk pages
    links:
      - {rel: prior-art, to: prior/layout-sqlite}
  bitmap:
    title: One bit per page or block in bitmap pages
    links:
      - {rel: prior-art, to: prior/layout-block-grid}
  extent-lists:
    title: Free extents as (offset, length) lists sorted by size
    links:
      - {rel: prior-art, to: prior/layout-block-grid}
---
Page allocation for every heap and index of the database.

## layout/alloc/freelist

SQLite: trunk pages of 4-byte page numbers (next trunk, count, leaves).
O(1) allocate and free of single pages; no contiguity.

```
trunk page:  u32 next_trunk | u32 n | u32 leaf[n] | unused
```

## layout/alloc/bitmap

TigerBeetle free set: one bit per grid block, held in memory and
written as blocks at checkpoint. A 1 TiB file of 8 KiB pages is
2^27 bits, 16 MiB of bitmap. Finding runs of free pages for extents is
a bit scan.

## layout/alloc/extent-lists

WiredTiger: `alloc`, `avail`, `discard` extent lists per checkpoint,
best fit from a size-sorted skiplist. Suits variable-size pages and
blobs. https://source.wiredtiger.com/develop/arch-block.html
