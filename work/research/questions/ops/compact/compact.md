---
title: How the single file is compacted
pick: some
raised-by: [layout/files/shrink/reuse-only]
options:
  offline-copy:
    title: Write every live version into a new file and swap, with the database closed
    links:
      - {rel: prior-art, to: prior/ops-sqlite-pragma}
  online-copy:
    title: Copy into a new file while serving, replay the log written meanwhile, then switch
  heap-relocate:
    title: Move one constructor heap's pages toward the start, rewriting its union-index entries, then truncate
---
Settled: freed pages are reused and the file shrinks only on an
explicit compaction. Heap pages are named by TIDs in union-index
entries, so moving a page rewrites entries.

## ops/compact/offline-copy

SQLite `VACUUM` and `VACUUM INTO`. Also the way to change a
format-fixed setting (`ops/config/scope/format`). Needs free space
equal to the live data.

```
$ adt compact app.db --into app.compact.db
```

## ops/compact/online-copy

Copy by scanning each heap at a snapshot, build indexes by merging
sorted runs (settled `btree/build/merge-runs`), then apply the redo
log since the snapshot and switch at a quiescent point. A second file
breaks "one file" for the duration.

## ops/compact/heap-relocate

Like a retag move (settled atomic move between heaps), but within one
heap: per heap, so a compaction pass is bounded by one constructor.
Each moved version gets a new TID and new index entries, which SSI
locks and snapshots must follow as for a retag.
