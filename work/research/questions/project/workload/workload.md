---
title: The workload the engine is built for
pick: one
raised-by: [substrate/new-engine]
options:
  embedded-oltp:
    title: Application database inside one process, as SQLite
    links:
      - {rel: prior-art, to: prior/engine-embedded-dbs}
      - {rel: gains, to: engine/embedded-server}
      - {rel: gains, to: layout/files/single-file}
  server-oltp:
    title: Shared transactional server for many clients, as Postgres
    links:
      - {rel: gains, to: engine/ssi}
      - {rel: gains, to: buffer/io-path/direct}
  htap:
    title: Transactions and analytic scans over the same data
    links:
      - {rel: prior-art, to: prior/exec-umbra}
      - {rel: gains, to: exec/parallel/morsel}
  analytics:
    title: Read-mostly analytics over bulk-loaded data, as DuckDB
    links:
      - {rel: prior-art, to: prior/duckdb-union}
      - {rel: harms, to: engine/ssi}
    assessment: {grade: costly, why: "SSI, B-link trees, per-row MVCC and full-page images are settled and are OLTP machinery"}
---
Which workload wins when two conflict. Never decided, yet settled
rulings already lean: SSI, B-link trees with OLC, per-row MVCC,
next-key locks, redo with full-page images and direct I/O presume
transactional point reads and writes; morsel parallelism, Arrow
batches, adaptive compilation and per-constructor statistics presume
scans; one file, an embeddable core and Linux-first with portable I/O
presume an application database. The answer sets the default page
size, the benchmark the planner is tuned for and what the spec promises
about latency.

```
type Identity = EmailPass { user: Key users, email: Text, hash: Bytes }
              | Google { user: Key users, sub: Text }
relation identities : Identity key id
-- OLTP shape: one key probe, one retag
from i in identities where i.id == id select i
-- analytic shape: a scan of every heap
from i in identities group i by tag(i) into g select { t: g.key, n: count(x for x in g) }
```

## project/workload/embedded-oltp

Many short transactions from one application, data local to it,
no administrator. SQLite positions itself as local storage for
applications and devices, not a shared repository, and points users
with many concurrent writers or very large data at client/server
engines (https://www.sqlite.org/whentouse.html). The server wrapper
of `engine/embedded-server` exists for remote access, not for scale.

## project/workload/server-oltp

Hundreds to thousands of sessions, many concurrent writers, data
larger than memory. Postgres shape. The settled concurrency stack
(SSI, readers that never block, group commit) fits it directly.

## project/workload/htap

One engine answers point transactions and analytic scans on fresh
data, as HyPer and Umbra claim (unverified for Umbra's OLTP numbers).
Per-constructor row heaps serve the first; morsel-driven vectorized
scans the second.

```
-- registered OLTP write and a registered report over the same heaps
update i in identities where i.id == id
set match i
  | EmailPass e -> Google { user: e.user, sub: sub }
  | Google g    -> g
from u in users select { id: u.id, n: count(i for i in u.<identities.user) }
```

## project/workload/analytics

Bulk loads, few writers, scans dominate. DuckDB's niche. Row heaps,
B-link trees and SSI become overhead; a columnar layout
(`storage/columnar`) was not chosen.
