---
title: Executing a match when the constructor is known only per row
pick: some
raised-by: [[query/construct, sum/value], exec/probe/fuse]
options:
  sel-vectors:
    title: Split each batch into one selection vector per tag
    links:
      - {rel: prior-art, to: prior/exec-x100}
  branch:
    title: Branch on the tag per row inside fused code
    needs-one-of: [exec/model/compiled, exec/model/adaptive]
  scatter:
    title: Scatter rows into per-constructor buffers, then run arm pipelines
---
Relation-level matches over a scan know the constructor per pipeline.
A value-level sum field, a union index probe, or a merge over several
heaps produces rows whose constructor differs row by row.

## exec/interleaved/sel-vectors

```
tag  [0, 1, 1, 2, 0]   ->  sel[0] = [0, 4]  sel[1] = [1, 2]  sel[2] = [3]
arm Courier over sel[0]; arm Post over sel[1]; arm Pickup over sel[2]
results scattered back to positions, so row order survives
```

X100 primitives all accept a selection vector. One pass over the tag,
then each arm is an ordinary kernel call. Cost grows with arms that
have few rows per batch.

## exec/interleaved/branch

`match tag { 0 => .., 1 => .., _ => .. }` per row in generated code.
Cheap with few arms and predictable tags; mispredicts with mixed tags
(unverified magnitude).

## exec/interleaved/scatter

Materialize per-tag buffers (a partitioning step), then each arm is a
pipeline over its buffer. Order is lost unless a position column is
kept.
