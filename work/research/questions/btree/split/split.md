---
title: How a full leaf picks its split point
pick: any
raised-by: [btree/structure/blink]
options:
  short-pivot:
    title: Among near-balanced split points, pick the one giving the shortest pivot
    links:
      - {rel: prior-art, to: prior/btree-nbtree}
  rightmost-fill:
    title: The rightmost leaf splits leaving the left page 90% full
    links:
      - {rel: prior-art, to: prior/btree-nbtree}
  dup-lopsided:
    title: Keep runs of one key on one page, accepting a lopsided split
  contention:
    title: Split a page hot with writers to unrelated entries
---
Layers on the split routine. Sum PKs drawn from one sequence insert at
the right edge of the union index whatever the constructor.

## btree/split/short-pivot

Postgres balances bytes, then moves within a window to where the first
differing attribute is earliest; with `(key, ctor, tid)` this is
usually a key boundary, so the pivot is just the key.

## btree/split/rightmost-fill

Postgres fillfactor: leaf 90, internal 70, single-value 96; plus a
cached rightmost-leaf fastpath that skips the descent for ascending
keys.

## btree/split/dup-lopsided

Postgres: when a page is all one key, assume ascending TIDs and leave
the left page mostly full. In a union index ascending TIDs hold only
per constructor, since each heap numbers its own blocks.

## btree/split/contention

Alhomssi and Leis: track latch contention per page; a page where
threads update different entries splits between the hot entries.
