---
title: Where old row versions live
pick: one
raised-by: [engine/own-pager]
options:
  in-heap:
    title: Every version a tuple in its constructor's heap, vacuum removes dead ones
  undo-store:
    title: Newest version in place, older ones in an undo log
  delta-memory:
    title: In-place updates with in-memory undo buffers for live snapshots
    links:
      - {rel: prior-art, to: prior/engine-embedded-dbs}
---
Batched vacuum per sum was settled, which presumes dead versions or dead
index entries left behind by updates.

## engine/in-heap

Postgres. A retag leaves the dead version in the old heap; vacuum per
sum collects it with the other heaps' dead TIDs.

## engine/undo-store

InnoDB, Oracle: readers rebuild old versions from undo. Index entries
for deleted keys are delete-marked and purged later, so batched purge
still applies. A retag's old version is in the undo of the old heap.

## engine/delta-memory

DuckDB, HyPer (Neumann et al. SIGMOD 2015): old values kept in memory
while a snapshot needs them; nothing to vacuum on disk.
