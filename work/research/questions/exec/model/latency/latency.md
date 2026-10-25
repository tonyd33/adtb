---
title: Compilation time dominates short queries
pick: any
raised-by: [exec/model/compiled]
options:
  at-register:
    title: Compile registered queries when they are registered
    needs: [evolution/breaks/invisible/registered]
  code-cache:
    title: Cache generated code per query and schema version
  live:
    title: Live with compile latency on every first call
    excludes: [exec/model/latency/at-register, exec/model/latency/code-cache]
links:
  - {rel: rivals, to: exec/model/adaptive}
---
A pgAdmin catalog query ran in under 1 ms in HyPer and took 54 ms to
compile with LLVM optimizations; the largest TPC-DS query took 911 ms
(Kohn et al. ICDE 2018). Point lookups over a sum are this shape.

## exec/model/latency/at-register

Hekaton natively compiled procedures (unverified) and Kersten et al.
sec. 8.2: stored procedures move code generation ahead of time.
Registered queries are known before any call, so registration
compiles; only ad hoc queries pay at run time.

```
$ adt register queries/
login_key  ok  plan#a1  code 38 KiB  compiled 41 ms
```

## exec/model/latency/code-cache

Generated code keyed by `(query, schema version, plan)`; constants
stripped into parameters. Kohn et al.: helps repeats, not first calls.

## exec/model/latency/live

Every process start pays compilation per query.
