---
title: What invalidates a cached plan
pick: some
raised-by: [exec/plan-cache/at-register, exec/plan-cache/first-call, exec/plan-cache/custom-generic]
options:
  schema:
    title: A change to any sum the plan reads
  stats:
    title: Statistics drifting past a threshold
  params:
    title: Parameter values outside the range the plan was costed for
---
Postgres replans a prepared statement on DDL to its objects, on
updated statistics, and on a `search_path` change.

## exec/replan/schema

The schema-change recheck of registered queries also replans them:
`add constructor Identity.Apple` adds a heap that a non-match query over
`identities` must read. Keyed by the version of each sum read, not a
global version, so unrelated migrations keep plans.

## exec/replan/stats

Replan when a heap's row count changes by a factor (e.g. 2x,
unverified threshold), so a constructor that grew from empty is no
longer planned as tiny.

## exec/replan/params

Keep several plans per query keyed by parameter selectivity buckets
(unverified precedent; SQL Server parameter-sensitive plans).
