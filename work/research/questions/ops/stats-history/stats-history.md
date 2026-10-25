---
title: How long execution statistics last
pick: one
raised-by: [ops/query-stats/by-name, ops/query-stats/fingerprint]
options:
  memory:
    title: Counters in memory since start, bounded, lost on restart
    links:
      - {rel: prior-art, to: prior/ops-pg-stats}
  checkpointed:
    title: Counters in memory, written on clean shutdown, reset after a crash
    links:
      - {rel: prior-art, to: prior/ops-pg-stats}
  windowed:
    title: Counters aggregated in time windows and kept in the file for a retention period
    links:
      - {rel: prior-art, to: prior/ops-query-store}
---
Plan regressions are diagnosed by comparing before and after. A
counter that resets on restart cannot show what a plan did yesterday.

## ops/stats-history/memory

pg_stat_statements without its file; MySQL performance_schema
summaries.

## ops/stats-history/checkpointed

Postgres cumulative statistics: kept in `pg_stat/` across clean
restarts, reset after crash, base backup or PITR.

## ops/stats-history/windowed

Query Store: runtime statistics per interval, written asynchronously,
size-capped. In the single file the windows are an engine-owned heap;
writing them must not enter the user's serializable history.

```
from w in sys::query_stats_window
where w.query == queries::login_key and w.start > now() - 7d
select { start: w.start, plan: w.plan, p99_ms: w.p99_ms }
```
