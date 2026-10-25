# dqlite (Canonical)

kind: system
home: https://canonical.com/dqlite

## Replication

- Replicates WAL frames, not SQL: the leader collects a write transaction's page writes into one Raft entry; state machine is the database file image plus WAL image. One write transaction at a time (SQLite WAL write lock). Success returned once a quorum persisted the entry. [V 2026-10-25] https://canonical.com/dqlite/docs/explanation/replication
