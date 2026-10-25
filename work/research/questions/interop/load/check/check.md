---
title: Constraint checks on a large change set
pick: any
raised-by: [interop/load/unit/one-set, interop/load/unit/staged]
options:
  set-at-a-time:
    title: Each constraint's delta query runs once over the whole delta, as a join
    links:
      - {rel: variant-of, to: constraint/enforce/delta}
  build-time:
    title: Uniqueness is checked while the index is built from sorted runs
    links:
      - {rel: gains, to: btree/build/merge-runs}
  unchecked:
    title: A trusted load skips checks and marks constraints unvalidated
    excludes: [constraint/timing/multi-assign]
    assessment: {grade: broken, why: a committed state may violate a constraint}
  live:
    title: Live with per-row probes on bulk loads
    excludes: [interop/load/check/set-at-a-time, interop/load/check/build-time, interop/load/check/unchecked]
---
Checked row by row, a load of N rows into a sum with an FK and two
uniques does 3N index probes. Postgres `COPY` checks per row
(https://www.postgresql.org/docs/current/sql-copy.html); `pg_dump`
avoids it by creating indexes and FKs after the data
(https://www.postgresql.org/docs/current/app-pgdump.html).

## interop/load/check/set-at-a-time

A constraint is a query that must return nothing
(`constraint/form/empty`), so its delta form over `inserted` is a
query too: the FK check is an anti-join of the delta against the
target's key index, planned and run vectorized like any query. One
plan, one pass, any delta size.

```
from i in inserted(identities) where not exists u in users where i.user == key u select i
```

## interop/load/check/build-time

Into an empty or rebuilt index, uniqueness falls out of the sorted
merge: equal adjacent keys are the violation report.

## interop/load/check/unchecked

Postgres `NOT VALID` constraints and `--disable-triggers` restores.
Contradicts every statement boundary being consistent.

## interop/load/check/live

N probes; correct, slow.
