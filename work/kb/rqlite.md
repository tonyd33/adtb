# rqlite

kind: system
home: https://rqlite.io/

## Replication

- Raft (hashicorp/raft); the Raft log stores committed SQL statements, applied identically on every node to a local SQLite; periodic snapshot and log truncation. [V 2026-10-25] https://rqlite.io/docs/design/
- Statement replication needs special handling of non-deterministic functions (RANDOM(), time). [S 2026-10-25] same
- Read levels: none (local, no checks), weak (local after checking it believes it is leader; brief stale window after deposition), linearizable, strong (applies every committed entry first), auto; `freshness` bounds staleness on none. [V 2026-10-25] https://rqlite.io/docs/api/read-consistency/
