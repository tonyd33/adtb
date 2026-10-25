---
title: How serialization failures are explained
pick: any
raised-by: [engine/ssi]
options:
  detail:
    title: The abort names the other transaction, its query and the conflicting constructor and key range
  log:
    title: A bounded log of conflicts, queryable as a system relation
    links:
      - {rel: prior-art, to: prior/ops-crdb}
  counters:
    title: Serialization failures and retries counted per registered query and constructor heap
---
Postgres reports `could not serialize access due to read/write
dependencies among transactions` with a reason code ("Canceled on
identification as a pivot, during commit attempt") and names neither
the other transaction nor the data. SSI aborts on a dangerous structure
of two rw-antidependencies; the engine knows both edges, and SIREAD
locks are tagged by constructor (settled `mvcc/ctor-tagged`), so the
conflict has a constructor and key range to report.

```
error serialization_failure
  role: pivot
  in:   T812 queries.link_account (web-2.3) read identities.Google key 4410..4410
  out:  T809 queries.retag_identity (admin-1.0) wrote identities.(Google | Apple) key 4410
  retry: safe
```

## ops/conflicts/detail

In the error itself, so the retry loop (settled `engine/retry/retry-block`)
can log it. Edges must be kept until the abort is raised; promoted
locks (tuple to page to heap) blur the range to a page or a heap.

## ops/conflicts/log

CockroachDB `transaction_contention_events`: blocking and waiting
transaction fingerprints, contending key, one SERIALIZATION_CONFLICT
event per transaction.

```
from c in sys::conflicts where c.at > now() - 1h
group c by c.ctor into g select { ctor: g.key, n: count(x for x in g) }
```

## ops/conflicts/counters

Aggregates only; shows which queries and constructors conflict, not
why.
