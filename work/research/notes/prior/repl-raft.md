---
title: Raft and its libraries
---
Ongaro and Ousterhout, ATC 2014: leader election, log replication,
joint-consensus membership, snapshots. Linearizable reads need a no-op
at term start and a majority heartbeat per read, or a lease that
assumes bounded clock skew. openraft (Rust, async) offers read index
and leader lease; dqlite replicates SQLite WAL frames through Raft;
rqlite replicates SQL statements. CockroachDB colocates the lease with
the Raft leader (leader leases). https://raft.github.io/raft.pdf
https://docs.rs/openraft/latest/openraft/
