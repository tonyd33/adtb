---
title: File count grows with constructors times forks
pick: any
raised-by: [layout/files/per-heap-file]
options:
  shared-small:
    title: Small heaps share one file in extents until they grow
  live:
    title: Live with a file per heap and fork
    excludes: [layout/files/many-files/shared-small]
---
A sum of 40 constructors with three forks each is 120 files before any
index. Each needs an fsync at checkpoint and a descriptor while open.

## layout/files/many-files/shared-small

A heap starts as extents inside a shared per-sum file and is moved to
its own file past a size threshold, as InnoDB general tablespaces hold
several tables in one file (unverified as a migration path).

## layout/files/many-files/live

Postgres runs with tens of thousands of relation files.
