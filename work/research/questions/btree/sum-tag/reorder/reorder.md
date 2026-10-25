---
title: Adding or reordering constructors renumbers declaration ranks in stored keys
pick: any
raised-by: [btree/sum-tag/decl-rank]
options:
  rewrite:
    title: The schema operator rebuilds every index whose key holds the sum
  append-only:
    title: New constructors are ranked after existing ones whatever the declaration order
  live:
    title: Live with stale ranks until a manual rebuild
    excludes: [btree/sum-tag/reorder/rewrite, btree/sum-tag/reorder/append-only]
    assessment: {grade: broken, why: the index returns rows in an order the language no longer has}
---
`evolution/add` of `Void` between `Sent` and `Paid` makes every stored
`Paid` key (rank 3) disagree with the new rank 4.

## btree/sum-tag/reorder/rewrite

The operator scans and re-encodes; cost proportional to rows holding
any constructor after the insertion point.

## btree/sum-tag/reorder/append-only

Ranks are assigned at creation and never move, so `Void` sorts after
`Paid` even if declared before it. Declaration order and sort order
part ways, as `stable-id` but with dense small numbers.

## btree/sum-tag/reorder/live

Accept.
