---
title: Removing dead index entries between vacuum passes
pick: any
raised-by: [btree/structure/blink, btree/structure/bw]
options:
  kill-bit:
    title: A scan that finds a row dead to all marks its entry dead
    links:
      - {rel: prior-art, to: prior/btree-nbtree}
  bottom-up:
    title: Before a split, check duplicates' rows and delete dead ones
    links:
      - {rel: prior-art, to: prior/btree-nbtree}
---
Per-sum vacuum is batched and periodic; between passes, version churn
fills leaves with dead entries and forces splits. Both layers below
need the heap an entry names, which the entry's `ctor` gives.

## btree/cleanup/kill-bit

Postgres LP_DEAD: set under a share latch like a hint bit; physically
removed when an insert would otherwise split the page. Not set if the
page changed since it was read.

## btree/cleanup/bottom-up

Postgres 14: when an update produces a logically unchanged duplicate,
group the page's duplicate entries by `(ctor, block)`, visit those heap
pages, delete entries whose rows are dead to all. A retag's old entry
`(7, EmailPass, tid)` beside `(7, Google, tid')` is such a duplicate.
