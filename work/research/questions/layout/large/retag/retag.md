---
title: Retag moves a row between heaps and its out-of-line values with it
pick: any
raised-by: [[layout/large/toast, union-index/retag/move]]
options:
  per-sum:
    title: One out-of-line store per sum, so pointers survive a move
  copy:
    title: Each constructor heap has its own store and retag copies the values
  live:
    title: Live with it
    excludes: [layout/large/retag/per-sum, layout/large/retag/copy]
---
Postgres has one TOAST table per table. With one heap per constructor,
a retag that keeps a 5 MB `body` field either re-points or rewrites it.

## layout/large/retag/per-sum

`toast(identities)` shared by every constructor heap of the sum; the
moved tuple keeps its 18-byte pointers. Vacuum of the sum (batched,
settled) also frees its chunks.

## layout/large/retag/copy

Retag writes the value again; cost proportional to its size.

## layout/large/retag/live

No move moves large values.
