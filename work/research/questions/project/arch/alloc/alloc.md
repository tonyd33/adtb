---
title: Memory allocation discipline
pick: one
raised-by: [buffer/memory/split]
options:
  static:
    title: Everything allocated at startup, nothing after
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
  budgeted:
    title: Fixed page pool; per-query arenas allocated on demand within a budget
  accounted:
    title: Global allocator with a counting wrapper that refuses past the limit
---
Settled: a fixed page pool plus bounded per-query memory. Open: whether
query memory is preallocated, carved on demand, or only counted.
TIGER_STYLE allocates all memory at startup and bounds every queue.

## project/arch/alloc/static

Every session, plan node and hash table slot exists from startup; a
query that needs more spills or fails.

## project/arch/alloc/budgeted

```
let arena = budget.arena(query_id, limit)?;   // Err(OutOfQueryMemory) past limit
let ht: HashTable<'_, K, V> = HashTable::new_in(&arena);
```

Arenas free whole at query end; the planner sizes builds against the
budget.

## project/arch/alloc/accounted

A `GlobalAlloc` wrapper counts per thread or task. Simple; the failure
point is wherever the allocation happens.
