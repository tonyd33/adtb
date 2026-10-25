---
title: How one query uses several cores
pick: one
raised-by: [substrate/new-engine]
options:
  serial:
    title: One thread per query
  exchange:
    title: Plan-driven parallelism through exchange operators
    links:
      - {rel: prior-art, to: prior/exec-volcano}
      - {rel: prior-art, to: prior/exec-datafusion}
  morsel:
    title: A dispatcher hands morsels of each pipeline to worker threads
    links:
      - {rel: prior-art, to: prior/exec-hyper}
      - {rel: gains, to: exec/flow/push}
---
Arms of a match read disjoint heaps, so they parallelise without
coordination until their shared sink.

## exec/parallel/serial

SQLite. Parallelism only across queries.

## exec/parallel/exchange

Volcano: operators written single-threaded, an exchange repartitions
streams. DataFusion: `RepartitionExec`, `target_partitions` defaults to
the core count; `UnionExec` of arms with N and M partitions has N + M
output partitions, no copying. The degree of parallelism is fixed at
plan time.

```
Union
  Repartition(8) <- Scan(email_pass)
  Repartition(8) <- Scan(google)
```

## exec/parallel/morsel

Leis et al. SIGMOD 2014: pinned workers take (pipeline, morsel) tasks,
morsels about 100,000 tuples (overhead negligible above 10,000), work
stealing, NUMA-local output areas; speedup over 30 on 32 cores. Each
arm's pipeline contributes morsels from its own heap.

```
tasks: [ (arm EmailPass, pages 0..12), (arm EmailPass, pages 12..24),
         (arm Google,    pages 0..3) ]          -> one shared sink
```
