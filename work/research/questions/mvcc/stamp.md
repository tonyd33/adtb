---
title: What a version records about its writers and what a snapshot is
pick: one
raised-by: [engine/ssi, engine/occ, engine/single-writer]
options:
  xid-clog:
    title: Writer transaction ids in the version, commit status in a commit log, snapshot as xmin, xmax and in-progress list
    links:
      - {rel: prior-art, to: prior/mvcc-pg-heap}
  commit-ts:
    title: Versions relabelled with the commit timestamp at commit, snapshot as one timestamp
    links:
      - {rel: prior-art, to: prior/mvcc-hyper}
      - {rel: prior-art, to: prior/engine-hekaton-calvin}
  csn-map:
    title: Writer transaction ids in the version, a map from id to commit sequence number, snapshot as one CSN
    links:
      - {rel: prior-art, to: prior/mvcc-undo-engines}
---
Every MVCC option needs a per-version stamp and a snapshot to compare
it with. The choice fixes the snapshot's size, the cost of commit, and
whether reads write (`mvcc/status`). Under one writer at a time
(`engine/single-writer`) the commit order is the writer order and any
of the three degenerates to a counter.

## mvcc/xid-clog

Postgres. Visibility of `xmin` for snapshot `s`: `xmin < s.xmin`, or
`xmin < s.xmax` and not in `s.xip`, and committed per pg_xact (2 bits
per xid, 32768 xids per 8 KiB page). In-progress must be checked
before the clog, since commit reaches pg_xact before the proc array.

```
snapshot { xmin: u32, xmax: u32, xip: [u32; n], curcid: u32 }
-- size grows with concurrent writers; xip searched per tuple
```

## mvcc/commit-ts

Hekaton and HyPer: an uncommitted version holds the writer's id in a
range above every timestamp (HyPer: ids from 2^63), replaced by the
commit timestamp after commit. Visible iff `begin <= s < end`. On disk
the relabel revisits every page the transaction wrote.

```
version { begin: u64, end: u64 }   -- id or timestamp, high bit says which
snapshot { ts: u64 }
```

## mvcc/csn-map

OrioleDB: versions keep the 64-bit OXid; an xid map gives its CSN once
committed; a snapshot is `{ csn, xmin, xlogptr, cid }`. Constant-size
snapshots without relabelling pages; the map lookup replaces the clog
lookup, and caching its answer in the version is a hint bit again.
