---
title: What execution statistics are keyed by
pick: one
raised-by: [lang/registered/named]
options:
  by-name:
    title: Registered queries by bundle and name; ad hoc queries by a fingerprint of their checked form
  fingerprint:
    title: Every query by a fingerprint of its normalised text, registered or not
    links:
      - {rel: prior-art, to: prior/ops-pg-stats}
---
pg_stat_statements has to recover query identity by hashing the
analysed tree with constants stripped; its `queryid` is not stable
across major versions. A registered query already has an identity: a
named, typed catalog object in a bundle.

```
from q in sys::query_stats where q.bundle == "web-2.3"
select { query: q.query, calls: q.calls, mean_ms: q.mean_ms, rows: q.rows,
         serialization_failures: q.serialization_failures, per_ctor: q.rows_by_ctor }
```

Per-constructor counters (rows read per arm, pruned arms) come free
from the per-arm plan.

## ops/query-stats/by-name

Keyed by (bundle, query, plan), so a replan starts a new row and plan
regressions are visible as a pair of rows. Ad hoc queries, which run
only in a scoped role, are grouped by a hash of the type-checked tree
with parameters abstracted.

## ops/query-stats/fingerprint

One mechanism for both kinds, as Postgres. Throws away the registered
identity and versions with it.
