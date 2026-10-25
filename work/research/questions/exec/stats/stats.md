---
title: What statistics the optimizer keeps for sums
pick: some
raised-by: [storage/table-per-ctor]
options:
  per-ctor:
    title: Row count and per-field statistics for each constructor heap
    links:
      - {rel: prior-art, to: prior/exec-pg-planner}
  sum-merged:
    title: Statistics of shared fields over the whole sum
    links:
      - {rel: prior-art, to: prior/exec-pg-planner}
  tag-freq:
    title: Tag frequencies and per-tag payload statistics for value-level sum fields
    needs: [sum/value]
---
Per-constructor heaps make the constructor count of a relation-level
sum free: it is the heap's row count. Fields inside value-level sums
need their own tag distribution.

```
stats identities.Google  rows 1.2M  domain: mcv [corp 0.31, ...] ndv 4100
stats identities (shared) user: ndv 0.9M
stats orders.shipping    tag: [Courier 0.62, Post 0.30, Pickup 0.08]
                         Post.zone: histogram [1..9]
```

## exec/stats/per-ctor

Postgres per-table `pg_statistic`: MCV list, histogram (default target
100 entries), n_distinct, correlation; `reltuples` per heap. An arm's
estimate uses only its heap's statistics.

## exec/stats/sum-merged

Postgres keeps `inherited = t` statistics for a whole hierarchy beside
each child's. Needed for hoisted joins and grouping on shared fields:
NDV over the sum is not the sum of per-heap NDVs. The sum key's NDV is
the total row count, since the union index makes it unique.

## exec/stats/tag-freq

The tag is a low-cardinality column whose MCV list is the whole
distribution. Per-tag statistics of payload fields estimate predicates
inside arms (`Post p -> p.zone > 3`).
