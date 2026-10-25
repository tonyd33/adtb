---
title: How a ctor-set index is built from existing rows
pick: one
raised-by: [encoding/native-cost/union-index]
options:
  merge-runs:
    title: Sort each heap's entries separately and merge the runs into a bottom-up build
  one-sort:
    title: Feed every heap of the set into one external sort
  retail:
    title: Insert entries one at a time
---
`add index (user) in identities.(Google | Facebook)`, rebuilds after
recovery (`btree/structure/persist`), and rewrites after schema
operators all build from several heaps. Postgres builds bottom-up from
sorted input, filling leaves to the fillfactor, then each level above
from the first keys of the level below (unverified details).

## btree/build/merge-runs

Each constructor heap is scanned and sorted independently, in
parallel, into a run of `(key, ctor, tid)`; one k-way merge streams
into leaves left to right. A heap that is already sorted by the key
(a sum PK from a sequence) skips its sort.

```
runs: EmailPass [(1,E,..),(5,E,..)]  Google [(2,G,..),(3,G,..)]  -> merge -> leaves
```

## btree/build/one-sort

One sorter over all heaps; one code path, no per-heap parallelism.

## btree/build/retail

No build code; each insert pays a descent and the result is about 70%
full.
