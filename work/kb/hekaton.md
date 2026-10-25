# Hekaton (SQL Server in-memory OLTP)

kind: system
home: https://www.microsoft.com/en-us/research/

## Concurrency and logging

- Optimistic MVCC with commit-time validation of reads and scans (Larson et al. VLDB 2011). [U]
- Logs only committed versions (redo); indexes are not logged and are rebuilt on recovery (Diaconu et al. SIGMOD 2013). [U]

## Versions (Larson et al. VLDB 2011)

- PVLDB 5(4) 2011, "High-Performance Concurrency Control Mechanisms for Main-Memory Databases". Each version has Begin and End fields holding a timestamp or a transaction id (one bit says which); a txn id in End is a write lock, in Begin marks an uncommitted version. Valid time [Begin, End), End infinity while current. [V 2026-10-25] https://vldb.org/pvldb/vol5/p298_per-akelarson_vldb2012.pdf sect. 2
- Records reachable only through indexes (lock-free hash tables), each index with its own pointer field per version; a table scan scans any index's buckets. [V 2026-10-25] sect. 2.1
- Serializable = read stability (validate read versions unchanged) plus phantom avoidance (rescan before commit). SI and read committed need no validation. Speculative reads of uncommitted versions take commit dependencies. [V 2026-10-25] sect. 1, 2.7
- Garbage collection is cooperative across threads. [V 2026-10-25] sect. 2
