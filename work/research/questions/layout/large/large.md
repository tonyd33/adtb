---
title: Where values too large for a page go
pick: one
raised-by: [engine/own-pager]
options:
  toast:
    title: Chunks in a side relation behind an inline pointer
    links:
      - {rel: prior-art, to: prior/layout-pg-page}
  overflow-chain:
    title: A linked chain of overflow pages after an inline prefix
    links:
      - {rel: prior-art, to: prior/layout-sqlite}
      - {rel: prior-art, to: prior/layout-innodb}
  blob-pages:
    title: One variable-size page or extent per value
    links:
      - {rel: prior-art, to: prior/layout-umbra}
  cap:
    title: No large values beyond a fraction of a page
---
A `Text`, `Bytes` or recursive value can exceed a page. Postgres tuples
never span pages; SQLite and InnoDB records spill.

## layout/large/toast

Postgres: over about 2 kB a row's widest fields are compressed, then
moved to a TOAST table as rows `(chunk_id, chunk_seq, chunk_data)` of
at most about 2000 bytes, behind an 18-byte pointer; 1 GB maximum.

```
inline:  varatt_external { rawsize, extsize|method, valueid, toastrelid }   18 B
toast:   (valueid, 0, bytes 0..1995) (valueid, 1, ...) ...
```

## layout/large/overflow-chain

SQLite keeps the first K bytes local and chains 4-byte next pointers;
InnoDB DYNAMIC moves the whole column off-page behind a 20-byte
pointer. Reading the tail is a walk of the chain.

## layout/large/blob-pages

Umbra: a value is one page of the smallest size class holding it.
LeanStore's large-object work stores blobs as extents with a small
inline descriptor (unverified detail). Needs an allocator of runs
(`layout/alloc/extent-lists` or `layout/alloc/bitmap`).

## layout/large/cap

Values over a limit are refused; applications store files elsewhere.
