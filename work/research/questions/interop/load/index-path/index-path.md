---
title: How a large load maintains the union indexes
pick: some
raised-by: [interop/load/source, interop/load/command, interop/load/append-api]
options:
  bulk-build:
    title: Into an empty constructor set, the index is built afterwards from sorted runs
    needs: [btree/build/merge-runs]
  sorted-merge:
    title: Into a populated index, the delta's entries are sorted and inserted in key order
  retail:
    title: Each row's entries are inserted as it is written
---
A cost choice the planner makes per load, not syntax. Union index
entries carry the constructor id after the key
(`btree/entry-order/after-key`).

## interop/load/index-path/bulk-build

Heaps filled first, then `btree/build/merge-runs` builds bottom-up
with uniqueness checked in the merge. The index is invisible until
the change set commits.

## interop/load/index-path/sorted-merge

Sorting the delta makes consecutive inserts land on the same leaf;
each leaf is visited once and splits fill left to right.

## interop/load/index-path/retail

One descent per entry; leaves end about 70% full (unverified).
