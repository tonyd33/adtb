---
title: When underfull index pages are merged
pick: one
raised-by: [btree/structure/blink]
options:
  empty-only:
    title: Only empty pages are removed, never merged
    links:
      - {rel: prior-art, to: prior/btree-nbtree}
  xmerge:
    title: Lazily compact groups of up to K siblings into one fewer page
  half-full:
    title: Merge or rebalance when a page drops below half full
---
Vacuum and retag churn leave sparse leaves. Alhomssi and Leis (CIDR
2021): random inserts give about 70% fill, sorted inserts 50%, and
industrial B-trees free a node only when it is empty.
https://www.cidrdb.org/cidr2021/papers/cidr2021_paper21.pdf

## btree/merge/empty-only

Postgres: an empty leaf is unlinked from its parent (half-dead), then
from its siblings, and recycled once no snapshot older than the
deletion remains. The rightmost page of a level is never deleted, so
height never shrinks.

## btree/merge/xmerge

For X from 2 to K (K = 5 found good), find X adjacent siblings with at
least one page of total free space, shift entries right, free the
leftmost. Parent latched exclusively during compaction.

## btree/merge/half-full

Textbook B-tree. Merges race with B-link right-link readers and need
the deletion protocol for every merge, not only for empty pages.
