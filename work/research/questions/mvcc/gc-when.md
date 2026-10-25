---
title: When reclamation work runs
pick: some
raised-by: [engine/in-heap, engine/undo-store, engine/delta-memory]
options:
  background:
    title: Periodic background passes per sum
    links:
      - {rel: prior-art, to: prior/mvcc-pg-heap}
      - {rel: prior-art, to: prior/mvcc-undo-engines}
  on-access:
    title: Any reader or writer holding a page or chain prunes what it can there
    links:
      - {rel: prior-art, to: prior/mvcc-pg-heap}
      - {rel: prior-art, to: prior/mvcc-steam}
  at-commit:
    title: Each commit frees whole transactions' version buffers below the horizon
    links:
      - {rel: prior-art, to: prior/mvcc-hyper}
      - {rel: prior-art, to: prior/mvcc-umbra}
links:
  - {rel: prior-art, to: prior/mvcc-wu-2017}
---
Detection, unlinking and freeing can run in different places. The union
index entries of dead versions are removed only by the batched per-sum
pass; the options here differ in what is freed before it.

## mvcc/background

Postgres autovacuum, InnoDB purge threads (default 1, or 4 above 16
CPUs; 300 undo pages per batch). The per-sum vacuum scans every
constructor heap, collects dead `(ctor, tid)`, then one union index
pass. Hekaton's background GC needs it for "dusty corners" no reader
visits.

## mvcc/on-access

HOT pruning: a page access that gets a cleanup lock frees heap-only
tuples and redirects chain roots without touching any index. Steam:
the thread updating a chain prunes it while holding the chain lock.
Frees heap space early but leaves line pointers the union index still
names.

## mvcc/at-commit

HyPer and Umbra: versions live in the writer's undo buffer; after every
commit, buffers of transactions older than the horizon are unlinked and
freed together. Wu et al. found transaction-level GC fastest with the
smallest footprint.
