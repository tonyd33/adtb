---
title: An approximate index has no defined predicate for serializable reads
pick: any
raised-by: [interop/vector/core-hnsw, interop/vector/ext]
options:
  whole-lock:
    title: A nearest-neighbour read takes a SIREAD lock on the whole index
    links:
      - {rel: variant-of, to: interop/index-am/duties/whole-lock}
  approx-exempt:
    title: Approximate reads are declared outside serializability and banned from writes' inputs
  live:
    title: Live with phantom-prone nearest-neighbour reads
    excludes: [interop/vector/isolation/whole-lock, interop/vector/isolation/approx-exempt]
---
SSI needs the read predicate to detect a concurrent insert that would
have changed the result (`mvcc/next-key`). "The ten nearest" has no
range; HNSW results also depend on graph state, so two runs at one
snapshot can differ.

## interop/vector/isolation/whole-lock

Every concurrent insert into `chunks` conflicts with every search.

## interop/vector/isolation/approx-exempt

An approximate result is typed `Approx` and cannot feed a change set.

## interop/vector/isolation/live

Write skew through similarity.
