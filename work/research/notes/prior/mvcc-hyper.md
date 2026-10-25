---
title: HyPer MVCC (Neumann, Mühlbauer, Kemper SIGMOD 2015)
---
In-place updates, before-image deltas in the writer's undo buffer,
newest-to-oldest chain from a per-record VersionVector. Uncommitted
deltas carry a transaction id above 2^63, relabelled to the commit
timestamp at commit. Serializability by precision locking: logged read
predicates are checked against the writes of transactions that
committed during the reader's life. Index-key updates are delete plus
insert, both entries kept until GC. GC at every commit.
https://db.in.tum.de/~muehlbau/papers/mvcc.pdf
