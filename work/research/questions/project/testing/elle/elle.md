---
title: Isolation checking needs reads that reveal version order
pick: any
raised-by: [project/testing/isolation-check]
options:
  text-append:
    title: Append to a Text field and read the whole value
  row-per-element:
    title: Insert one row per appended element and read the set
  live:
    title: Check only anomalies inferable from reads of registers
    excludes: [project/testing/elle/text-append, project/testing/elle/row-per-element]
---
Elle's strongest workload appends unique elements to lists, so every
read shows the order of writes before it (traceability). Values here
have no collections (`model/no-collections`) and relations are sets.

## project/testing/elle/text-append

Jepsen tested TiDB with SQL `CONCAT` over TEXT fields for the same
purpose (Elle paper, §TiDB).

```
type Cell = { k: I64, v: Text }
relation cells : Cell key k
update c in cells where c.k == k set { c with v: c.v ++ "," ++ show(e) }
from c in cells where c.k == k select c.v
```

## project/testing/elle/row-per-element

```
relation elems : { k: I64, e: I64 } key (k, e)
insert elems { k: k, e: e }
from x in elems where x.k == k select x.e
```

A set shows which writes are visible but not their order: recoverable,
not traceable. Fewer anomalies inferable, but exercises the union
index and range locks instead of one row.

## project/testing/elle/live

Read-write registers only; Elle infers less from them.
