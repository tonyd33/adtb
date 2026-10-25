---
title: Which page of a constructor heap an insert goes to
pick: one
raised-by: [engine/own-pager]
options:
  fsm:
    title: A free space map with a coarse free-space value per page
  append:
    title: Always the heap's last page and reclaim space only by compaction
    links:
      - {rel: prior-art, to: prior/layout-umbra}
  slot-bitmap:
    title: Fixed-slot heaps keep a free-slot bitmap per page and a list of pages with free slots
    needs-one-of: [layout/slots/fixed-slot, layout/slots/by-schema]
---
Per-constructor heaps hold rows of one schema, so every row of a
fixed-width constructor is the same size and "does it fit" is "is a
slot free".

## layout/fill/fsm

Postgres: a separate fork, one byte per heap page in a tree of FSM
pages, the byte quantising free space in 1/256 steps of the page
(unverified details). https://www.postgresql.org/docs/current/storage-fsm.html

## layout/fill/append

Umbra: tids increase monotonically and nodes are filled, never split.
Deleted rows leave holes until a vacuum moves live rows, which changes
their tids and so rewrites union index entries.

## layout/fill/slot-bitmap

```
page header ... u64 free_slots[ceil(n/64)]
heap meta page: list of page numbers with a free slot
```

Insert pops a page from the list and the lowest free bit. Vacuum frees
a slot by setting its bit; no compaction ever needed in fixed-slot
pages.
