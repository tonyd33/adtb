---
title: What the spec fixes about the engine
pick: one
raised-by: [substrate/new-engine]
options:
  guarantees-only:
    title: Spec states guarantees over constructor relations, mechanism free
    links:
      - {rel: prior-art, to: prior/engine-pg-ssi}
  mechanism-fixed:
    title: Spec fixes concurrency control and storage mechanism
---
The spec can promise observable behaviour (isolation, durability, when
constraints are checked) or name the machinery that delivers it.

## engine/guarantees-only

Isolation is defined by forbidden phenomena over the logical model, as
Adya (MIT PhD 1999, https://pmg.csail.mit.edu/papers/adya-phd.pdf, unverified URL)
defines levels by dependency-graph cycles independent of locking. Reads
and writes are on constructor relations; a retag is a write to two of
them; a read of `identities` is a predicate read over all of them. The
union index, heaps and TIDs never appear.

```
-- spec: a committed history over identities.EmailPass, identities.Google, ...
-- has an acyclic DSG (serializable)
from i in identities where i.user == u select i.id
```

## engine/mechanism-fixed

The spec states that the engine is MVCC with SSI over per-constructor
heaps, as SQLite's file format and locking are part of its documented
contract (https://www.sqlite.org/fileformat2.html). Alternative engines
are non-conforming.
