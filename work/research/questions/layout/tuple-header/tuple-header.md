---
title: The per-version header on heap tuples
pick: one
raised-by: [engine/in-heap]
options:
  pg-header:
    title: Postgres-style 23-byte header with attribute count and infomasks
    links:
      - {rel: prior-art, to: prior/layout-pg-page}
  lean:
    title: Fixed 24-byte header with 64-bit xids and no attribute count or header length
---
Old versions live in the constructor's heap (engine/in-heap), so each
tuple carries its inserter, deleter and successor.

## layout/tuple-header/pg-header

```
0   u32 xmin
4   u32 xmax
8   u32 cid / xvac
12  u48 ctid            (block u32, slot u16)
18  u16 infomask2       attribute count + flags
20  u16 infomask
22  u8  hoff            then null bitmap, padding to MAXALIGN
```

`infomask2`'s count and the null bitmap exist for rows of differing
width and NULLs; neither occurs here. 32-bit xids need freezing before
wraparound. https://www.postgresql.org/docs/current/storage-page-layout.html

## layout/tuple-header/lean

```
0   u64 xmin
8   u64 xmax            0 = live
16  u48 ctid            successor in this heap
22  u16 flags           committed/aborted hints, moved, ...
24  body                at a fixed offset for every row of the heap
```

The heap's schema (with the page's format version) fixes the body
layout, so no count or header length. 64-bit xids drop wraparound
freezing; the command id lives in the writing transaction's local state
for its own versions (unverified as sufficient).
