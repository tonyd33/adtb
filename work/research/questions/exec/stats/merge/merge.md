---
title: Sampled statistics do not merge across heaps
pick: any
raised-by: [[exec/stats/sum-merged, exec/stats-form/sampled]]
options:
  resample:
    title: Sample the sum as a whole on analyze
  live:
    title: Live with sum statistics stale relative to heaps
    excludes: [exec/stats/merge/resample]
---
Histograms and n_distinct from separate samples cannot be combined
into the sum's distinct count. Mergeable sketches (`exec/stats-form/sketch`)
combine by union.

## exec/stats/merge/resample

Postgres analyzes an inheritance parent with a sample across children
for the `inherited` row (unverified mechanism).

## exec/stats/merge/live

Sum-level estimates use the last whole-sum analyze.
