---
title: Granularity of SIREAD locks on union index ranges
pick: one
raised-by: [engine/ssi]
options:
  leaf-page:
    title: Lock every leaf page the scan covered
    links:
      - {rel: prior-art, to: prior/mvcc-pg-ssi-internals}
  next-key:
    title: Lock the key ranges between the entries the scan saw
---
SIREAD locks never block; a writer checks for them and records a
rw-antidependency. Heap-side, tuple locks name `(ctor, tid)` and
promote to the heap page, then to the constructor heap; inserts
conflict only with heap-level locks. Index-side locks catch phantoms.

## mvcc/leaf-page

Postgres B-tree: an insert into a locked leaf page conflicts. Page
splits must copy locks to the new page. Every entry on the page is a
false-positive target, including those of constructors the query did
not read.

## mvcc/next-key

Next-key ranges as in InnoDB's S2PL gap locks, planned but not built
for Postgres SSI (Ports and Grittner sect. 5.2.1). Lock entries are
`(index, lo, hi)`; a lock table keyed by range needs an interval
structure, and promotion to page or index level under memory pressure.
