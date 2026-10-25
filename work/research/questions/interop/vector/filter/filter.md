---
title: Filters after an approximate scan return too few rows
pick: any
raised-by: [interop/vector/core-hnsw, interop/vector/ext]
options:
  iterative:
    title: The scan resumes until enough rows pass the filter
    links:
      - {rel: prior-art, to: prior/interop-pgvector}
  in-traversal:
    title: The filter is a predicate evaluated during graph traversal
    links:
      - {rel: prior-art, to: prior/interop-usearch}
  per-ctor:
    title: One graph per constructor heap, so constructor filters are exact
    links:
      - {rel: rivals, to: index/ctor-set, for: "one index over a constructor set"}
  live:
    title: Live with short results
    excludes: [interop/vector/filter/iterative, interop/vector/filter/in-traversal, interop/vector/filter/per-ctor]
---
pgvector applies filters after the index scan; 0.8 adds iterative
scans (https://github.com/pgvector/pgvector). A constructor set is a
filter too.

## interop/vector/filter/iterative

`top 10` with a 1% selective filter reads until ten pass.

## interop/vector/filter/in-traversal

usearch accepts a predicate during search
(https://github.com/unum-cloud/usearch). The predicate is the
compiled `where`.

## interop/vector/filter/per-ctor

`chunks.(Para | Code)` probes two graphs and merges top-k. Departs
from one index per constructor set.

## interop/vector/filter/live

Ten requested, three returned.
