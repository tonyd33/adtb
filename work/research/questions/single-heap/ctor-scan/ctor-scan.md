---
title: Scanning one constructor reads every constructor's rows
pick: any
raised-by: [encoding/native-cost/single-heap]
options:
  cluster:
    title: Cluster the heap by (ctor, id)
  columnar:
    title: Pair with columnar storage, tag column skips other rows
    needs-one-of: [storage/columnar, storage/sparse-columnar]
  live:
    title: Live with it when constructors are of similar size
    excludes: [single-heap/ctor-scan/cluster, single-heap/ctor-scan/columnar]
---
`from g in identities.Google` over one heap filters by tag. A rare
constructor pays for a scan of the whole sum.

## single-heap/ctor-scan/cluster

Rows of one constructor are physically contiguous, as with Postgres
`CLUSTER` or an index-organized table keyed `(ctor, id)` (unverified as
a maintained order). Uniqueness of `id` alone still needs its own index.

## single-heap/ctor-scan/columnar

Per-granule tag statistics skip blocks without the constructor;
ClickHouse stores a granule of one discriminator as 3 values.

## single-heap/ctor-scan/live

No layout work.
