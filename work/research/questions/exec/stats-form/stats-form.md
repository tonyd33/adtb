---
title: How statistics are gathered
pick: some
raised-by: [substrate/new-engine]
options:
  sampled:
    title: Periodic analyze over a sample builds histograms and MCV lists
    links:
      - {rel: prior-art, to: prior/exec-pg-planner}
  sketch:
    title: Online reservoir samples and mergeable distinct-count sketches
    links:
      - {rel: prior-art, to: prior/exec-umbra}
  maintained:
    title: Exact row counts per heap and per tag maintained by writes
---
Constructor heaps multiply the number of statistics objects by the
constructor count; retag moves a row between two of them.

## exec/stats-form/sampled

Postgres `ANALYZE`; values go stale between runs and the planner
scales them to the current heap size.

## exec/stats-form/sketch

Umbra: online reservoir sampling plus updateable HyperLogLog per column
(CIDR 2020, sec. 3.2). HLL sketches merge by register-wise max
(unverified as Umbra's merge), so a sum's NDV is the merge of its
heaps'.

```
hll identities.Google.user  2^12 registers x 6 bit = 3 KiB  (unverified size)
hll identities.user = merge(EmailPass.user, Google.user, Apple.user)
```

## exec/stats-form/maintained

Per-heap counters bumped by insert, delete and retag (a retag moves one
from the old constructor to the new). Approximate under MVCC unless
versioned (unverified cost).
