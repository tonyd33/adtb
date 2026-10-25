---
title: How the engine allocates memory
pick: one
raised-by: [substrate/new-engine]
options:
  static:
    title: Everything allocated at startup, bounded queues, no allocation afterwards
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
  unified:
    title: One memory budget shared by the page pool and query operators, which spill
  split:
    title: A fixed page pool plus per-query arenas with per-operator limits
    links:
      - {rel: prior-art, to: prior/buffer-pg}
  general:
    title: General-purpose allocator with no engine-level budget
---
Decides whether running out of memory is a startup error, a spill, or
a crash, and whether the pool can grow.

## buffer/memory/static

TIGER_STYLE: all memory at startup, every queue and loop bounded, no
recursion. The pool is a fixed frame array; each concurrent operation
is a preallocated slot, so the slot count bounds concurrency.
https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md

```
startup:
  frames:   [Frame; pool_pages]     aligned 4096
  queries:  [QuerySlot; max_queries], each with arena: [u8; query_bytes]
  io:       [IoSlot; ring_depth]
```

## buffer/memory/unified

DuckDB: `memory_limit` (default 80% of RAM) covers cached blocks and
operator state; hash joins, sorts and aggregates spill to a temp
directory (default cap 90% of free disk).
https://duckdb.org/docs/2.0/configuration/overview.html

## buffer/memory/split

Postgres: `shared_buffers` fixed at start; each backend allocates
memory contexts freed with the query; `work_mem` per sort or hash node
(unverified details), so total is unbounded under concurrency.

## buffer/memory/general

malloc everywhere; OOM kills the process.
