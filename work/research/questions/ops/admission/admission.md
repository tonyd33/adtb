---
title: How the engine decides what runs when it is overloaded
pick: some
raised-by: [buffer/memory/split]
options:
  none:
    title: A connection limit only; every call runs at once
  slots:
    title: A fixed number of execution slots; calls queue for one
  queue:
    title: Work queues per resource ordered by priority, then transaction start
    links:
      - {rel: prior-art, to: prior/ops-crdb}
  grant:
    title: A call is admitted when its plan's memory grant fits the query budget
---
Settled: a fixed page pool plus bounded per-query memory. Bounded per
query does not bound the sum over concurrent queries. Queue and grant
stack: the queue orders waiting calls, the grant decides when the head
runs.

## ops/admission/none

Postgres `max_connections`; `work_mem` times concurrency is unbounded.

## ops/admission/slots

TigerBeetle-like: concurrency fixed at startup.

## ops/admission/queue

CockroachDB admission control: CPU and storage-write queues, critical
over regular over background work, lock holders first.

## ops/admission/grant

SQL Server memory grants (unverified detail: `RESOURCE_SEMAPHORE`
waits). A registered query's plan gives a memory estimate at
registration; admission waits until the sum of grants fits.

```
from p in sys::plans where p.query == queries::report select p.memory_grant   -- 64 MiB
```
