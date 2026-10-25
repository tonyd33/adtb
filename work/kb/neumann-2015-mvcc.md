# Neumann, Mühlbauer, Kemper: Fast Serializable MVCC for Main-Memory Database Systems

kind: paper
home: https://dl.acm.org/doi/10.1145/2723372.2749436

## Versions

- SIGMOD 2015, HyPer. Updates in place; the before-image of changed attributes goes to the updating transaction's undo buffer; a per-record VersionVector entry (null when unversioned) anchors a newest-to-oldest chain of deltas across undo buffers, bidirectional for GC. [V 2026-10-25] https://db.in.tum.de/~muehlbau/papers/mvcc.pdf sect. 2
- Timestamps: startTime from 0 upward, transactionID from 2^63 upward so uncommitted deltas look newer than every start; commitTime from the same counter as startTime. At commit, the txn's deltas are relabelled from its transactionID to its commitTime, all local to its undo buffer. [V 2026-10-25] sect. 2
- Visible version: copy the in-place record and undo deltas until v.pred = null or v.pred.TS = T or v.pred.TS < T.startTime. [V 2026-10-25] sect. 2.2
- Write-write: an update of an object whose newest delta carries an uncommitted transactionID aborts the updater, so the first chain pointer always leads to a committed version. [V 2026-10-25] sect. 2.3
- VersionedPositions: per 1024-record range a 32-bit synopsis of first (high 16 bits) and last (low 16 bits) versioned position; scans run branch-free over unversioned ranges. [V 2026-10-25] sect. 2.6

## Serializability

- Precision locking: a transaction logs read predicates per relation (64-bit comparison summaries per attribute, strings hashed); at commit it checks the undo buffers of transactions that committed during its lifetime: created versions against predicates (phantom), deleted objects against reads, updates by both before and after image. Validation cost is proportional to recent writes, not read set. Attribute-level variant (SR-AL) also logs unrestricted attribute reads to cut false positives. Predicates kept in a per-relation predicate tree. [V 2026-10-25] sect. 2.3
- Index nested loop join values are coarsened into ranges in the predicate log. [V 2026-10-25] sect. 2.3.1
- Commit: short exclusive critical section draws commitTime, validates, writes redo; timestamp relabelling after it with atomics. [V 2026-10-25] sect. 2.7

## Indexes and GC

- No predicate locks or timestamps in indexes. An update of an indexed attribute is delete plus reinsert, both kept in the index until GC. Unique PK insert aborts if the key exists in its snapshot, the latest committed version, or uncommitted. FK insert aborts on a concurrent, even uncommitted, delete of the target. [V 2026-10-25] sect. 2.5
- GC after every commit: find oldest visible transactionID, drop older committed txns from recentlyCommitted, unlink their undo buffers, tombstone them; memory reused once the oldest active transaction started after the tombstone. [V 2026-10-25] sect. 2.4
