---
title: Scanning the whole sum is an N-way union of heaps
pick: any
raised-by: [storage/table-per-ctor]
options:
  merge-append:
    title: Merge-append ordered scans of each heap
  live:
    title: Live with union scans
    excludes: [storage/sum-scan/merge-append]
---
`from i in identities` reads every constructor heap. An ordered scan by
`id` must merge N streams. Many small constructors means many small
files.

## storage/sum-scan/merge-append

As Postgres does for partitioned tables: each heap scanned in index
order, merged on the fly (unverified as directly applicable).

## storage/sum-scan/live

Unordered union; sort when needed.
