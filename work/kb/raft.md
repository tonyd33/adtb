# Raft (Ongaro, Ousterhout)

kind: paper
home: https://raft.github.io/raft.pdf

## Protocol

- USENIX ATC 2014, "In Search of an Understandable Consensus Algorithm". Leader election by randomized timeouts, log replication by AppendEntries (also the heartbeat), terms as a logical clock; Log Matching property. [V 2026-10-25] https://raft.github.io/raft.pdf
- Membership change by joint consensus: a transitional configuration needs majorities of both old and new. [V 2026-10-25] same, sect. 6
- Log compaction by snapshot; InstallSnapshot RPC sends chunks to lagging followers. [V 2026-10-25] same, sect. 7
- Linearizable reads without writing the log: the leader commits a blank no-op at term start, and before a read exchanges heartbeats with a majority; alternatively a heartbeat-based lease, which relies on bounded clock skew for safety. [V 2026-10-25] same, sect. 8
