# mvSQLite

kind: system
home: https://github.com/losfair/mvsqlite

## Design

- SQLite VFS over FoundationDB: page-level MVCC, optimistic fine-grained concurrency with BEGIN CONCURRENT-like semantics, time travel to any past snapshot. [V 2026-10-25] https://github.com/losfair/mvsqlite
