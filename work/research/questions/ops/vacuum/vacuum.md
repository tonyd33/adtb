---
title: What operators see and control of vacuum
pick: any
raised-by: [union-index/vacuum/batched]
options:
  progress:
    title: A progress relation per running vacuum, by sum, heap and phase
    links:
      - {rel: prior-art, to: prior/ops-pg-stats}
  trigger:
    title: An operator can start vacuum of one sum now
  throttle:
    title: Vacuum runs under an I/O and CPU budget that operators set
  background-off:
    title: Background vacuum can be switched off per sum
---
Settled: vacuum is per sum, one union-index pass over dead TIDs
collected from every constructor heap; per-sum background vacuum plus
on-access pruning; no wraparound, so no freezing. Postgres exposes
`pg_stat_progress_vacuum` and per-table autovacuum settings.

## ops/vacuum/progress

```
from v in sys::vacuums
select { sum: v.sum, phase: v.phase, heaps_done: v.heaps_done, dead_tids: v.dead_tids,
         index_pass_pct: v.index_pass_pct }
-- type VacuumPhase = ScanHeaps { heap: CtorId } | IndexPass | Truncate
```

## ops/vacuum/trigger

```
vacuum identities
```

## ops/vacuum/throttle

Postgres `vacuum_cost_limit` and `vacuum_cost_delay` (unverified
detail); here a share of the I/O ring.

## ops/vacuum/background-off

Per-table `autovacuum_enabled = off` in Postgres. For bulk loads.
