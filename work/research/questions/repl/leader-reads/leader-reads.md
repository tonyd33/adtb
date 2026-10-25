---
title: How the primary knows it is still primary when it serves a read
pick: one
raised-by: [repl/failover/raft, repl/failover/vsr, repl/failover/external-lease]
options:
  read-index:
    title: Confirm leadership with a quorum round per read batch
    links:
      - {rel: prior-art, to: prior/repl-raft}
  lease:
    title: A leader lease; reads are local while it lasts
    links:
      - {rel: prior-art, to: prior/repl-raft}
  serializable-only:
    title: Promise serializable, not strict; a deposed primary may serve a stale read
---
Serializability allows a read-only transaction to see an older state;
strict serializability forbids it once a newer commit was
acknowledged. A deposed primary that has not heard of its successor
serves exactly such a read.

## repl/leader-reads/read-index

Raft sect. 8: a no-op at term start, then a majority heartbeat before
answering. One round trip per read batch, no clock assumption.

## repl/leader-reads/lease

Raft sect. 8 notes the lease relies on bounded clock skew.
CockroachDB leader leases colocate the lease with the Raft leader so
the leaseholder reads without a Raft round trip.

## repl/leader-reads/serializable-only

rqlite `weak`: the node checks only its own belief that it leads. The
spec says strict serializability holds only within one primary's
tenure, and a bookmark from the new primary is refused by the old.
