---
title: Distributing a join repeats the other input's work per arm
pick: any
raised-by: [exec/join/distribute, exec/join/cost]
options:
  shared-build:
    title: Build the other side's hash table once, probe it from every arm
    needs: [exec/flow/push]
  spool:
    title: Materialize the other side once and rescan it per arm
  live:
    title: Live with rebuilding per arm
    excludes: [exec/join/rebuild/shared-build, exec/join/rebuild/spool]
---
Distributed naively, `users` is scanned and hashed once per arm.

## exec/join/rebuild/shared-build

One build pipeline `Scan(user) -> HashBuild`; N probe pipelines
`Scan(heap_c) -> arm_c -> Probe -> sink`. In a push engine a hash table
is already a pipeline breaker several pipelines may read (Leis et al.
sec. 4.2 builds once, probes from many threads).

## exec/join/rebuild/spool

A shared subplan materialized to a temporary and read N times, as a
common table expression (unverified as an engine precedent).

## exec/join/rebuild/live

N scans and hash builds of the other side.
