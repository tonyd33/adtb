---
title: How a forward link names the successor in another heap
pick: one
raised-by: [engine/retag-chain/forward-pointer]
options:
  wide:
    title: Every version header carries the successor's constructor id and TID
  key-relookup:
    title: A retag sets a moved marker and the waiter re-finds the row by its sum key
    links:
      - {rel: prior-art, to: prior/mvcc-pg-heap}
      - {rel: prior-art, to: prior/mvcc-undo-engines}
  global-tid:
    title: A retag stores the successor TID in next; page ids are file-global
---
Postgres `t_ctid` is a 6-byte `(block u32, offset u16)`; a row moved to
another partition gets the magic `(InvalidBlockNumber, 0xfffd)` and no
destination (`prior/engine-pg-row-movement`). Per-constructor heaps
have one fixed schema, so the header needs no null bitmap and no
`t_hoff`: the data offset is a constant of the heap's format.

```
wide (24 bytes)                     key-relookup (20 bytes)
  0  u32 xmin                         0  u32 xmin
  4  u32 xmax                         4  u32 xmax
  8  u32 cid                          8  u32 cid
 12  u32 next.ctor                   12  u32 next.block   -- 0xFFFFFFFF: moved
 16  u32 next.block                  16  u16 next.offset  -- 0xfffd: moved
 20  u16 next.offset                 18  u16 infomask
 22  u16 infomask                    20  fields at catalog offsets
 24  fields at catalog offsets
```

## mvcc/link/wide

`next.ctor` equals the tuple's own constructor except after a retag.
Four bytes per version on every heap, used only by retags. Following
is one heap fetch.

## mvcc/link/key-relookup

A retag keeps the key and the key is unique across the sum, so the
waiter probes the sum's primary union index with the old version's key
and takes the entry whose xmin equals the old xmax. Costs an index
probe per follow, nothing per tuple. OrioleDB re-finds rows by primary
key and documents the failure mode below.

```
update i in identities.EmailPass where i.id == 7 set Google { gid: g }
-- waiter on (EmailPass, tid 9): xmax committed, next = moved
-- probe identity_pk (7) -> (7, Google, tid 4), xmin = old xmax: follow
```

## mvcc/link/global-tid

The version header's `next` is a 64-bit TID whose page id is global to
the file, so it names a successor in another constructor's heap
without a constructor id. The waiter follows `next` directly. A
delete and reinsert of the key is never mistaken for a successor, and
two retags of one row in one transaction stay distinct.
