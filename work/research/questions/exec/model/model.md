---
title: How a physical plan executes
pick: one
raised-by: [substrate/new-engine]
options:
  volcano:
    title: Tuple-at-a-time iterators
    assessment: {grade: costly, why: per-tuple virtual calls dominate in-memory work}
    links:
      - {rel: prior-art, to: prior/exec-volcano}
  vectorized:
    title: Interpreted operators over vectors with precompiled kernels
    links:
      - {rel: prior-art, to: prior/exec-x100}
      - {rel: prior-art, to: prior/duckdb-union}
      - {rel: prior-art, to: prior/exec-velox}
      - {rel: prior-art, to: prior/exec-datafusion}
  compiled:
    title: Each pipeline compiled to one fused machine-code loop
    needs: [exec/flow/push]
    links:
      - {rel: prior-art, to: prior/exec-hyper}
  adaptive:
    title: Pipelines start in a bytecode interpreter and compile when it pays
    needs: [exec/flow/push]
    links:
      - {rel: prior-art, to: prior/exec-umbra}
links:
  - {rel: prior-art, to: prior/exec-kersten}
---
Per-constructor heaps give every scan one fixed schema: field offsets,
widths and types are known when the plan is built, and no field is
NULL. Every model below can exploit that; they differ in where the
specialisation happens (precompiled kernels chosen once per pipeline,
or code generated per query). Value-level sum fields still need a tag
test per row.

```
from i in identities.Google where i.domain == "corp" select i.gid
-- scan(google heap) -> filter(domain) -> project(gid)
```

## exec/model/volcano

`next()` returns one row. MySQL on TPC-H Q1 spent 38 instructions at
IPC 0.8 (about 49 cycles) per addition and under 10% of time in the
arithmetic (Boncz et al. CIDR 2005, Table 2).

```
trait Op { fn next(&mut self) -> Option<Row>; }
```

## exec/model/vectorized

X100, DuckDB, Velox, DataFusion. `next()` returns a batch of about
1000 to 8192 rows; work is done by primitives over one typed column
with an optional selection vector. With a fixed schema per constructor
heap, the plan picks `filter_eq_utf8` once per pipeline, never per row.
Vector sizes 128 to 8K worked (X100 Fig. 10); Kersten et al. found
<64 and >64K degrade.

```
fn filter_eq_str(col: &StrCol, lit: &str, sel_in: &[u32], sel_out: &mut Vec<u32>)
```

## exec/model/compiled

HyPer (Neumann VLDB 2011): `produce`/`consume` generate one loop per
pipeline; values stay in registers between pipeline breakers. Typer
beat Tectorwise by 74% on Q1 and lost by 32% on Q9 (Kersten et al.
VLDB 2018, sec. 4.1). A fixed row layout per constructor lets the
generated loop load fields at constant offsets.

```
for row in google_heap.pages().rows() {          // generated
  if row.str_at(16) == "corp" { out.push(row.str_at(0)); }
}
```

## exec/model/adaptive

Umbra and Kohn et al. (ICDE 2018): generated IR runs first in a
bytecode VM; per pipeline the engine extrapolates remaining work from
morsel progress and compiles unoptimized or optimized code when the
estimate says it pays. Umbra is 3.0x HyPer on JOB, partly from not
compiling cheap queries (CIDR 2020, sec. 4).
