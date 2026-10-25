---
title: Query plans and nested results have no static bound
pick: any
raised-by: [project/arch/alloc/static]
options:
  bound-at-register:
    title: Registered queries get worst-case memory computed at registration
    needs: [exec/plan-cache/at-register]
  spill:
    title: Every operator spills to pages past its preallocated share
  live:
    title: Refuse queries exceeding the static pool
    excludes: [project/arch/alloc/unbounded/bound-at-register, project/arch/alloc/unbounded/spill]
---
TigerBeetle can preallocate because its eight operations are fixed.
Here users write queries: nested results, `group into`, `fix` and hash
builds grow with data.

## project/arch/alloc/unbounded/bound-at-register

Plan size is known at registration; data-dependent operators are
assigned a share and spill.

## project/arch/alloc/unbounded/spill

Hash joins, groups and fixpoints spill to temporary pages from the
pool.

## project/arch/alloc/unbounded/live

`OutOfQueryMemory` at run time.
