# Böttcher, Leis, Neumann, Kemper: Scalable Garbage Collection for In-Memory MVCC Systems (Steam)

kind: paper
home: https://www.vldb.org/pvldb/vol13/p128-bottcher.pdf

## Findings

- PVLDB 13(2) 2019 (VLDB, not SIGMOD). In HTAP mixed workloads GC is often the bottleneck: a long query holds the watermark, chains grow, reads slow, queries run longer ("vicious cycle"). [V 2026-10-25] https://www.vldb.org/pvldb/vol13/p128-bottcher.pdf sect. 1
- Survey table: Hekaton transaction-level, background txn map, ~1 min default, full O2N in relation; HyPer transaction-level at commit, undo log N2O delta; HANA tuple/txn/table, 1/10 s background interval GC, keeps oldest version in place; Peloton, ERMIA, Deuteronomy epoch based. [V 2026-10-25] Table 1
- Watermark GC (min start timestamp) cannot reclaim versions in the middle of a chain; interval GC can, keeping only versions some active snapshot needs. [V 2026-10-25] sect. 3

## Steam design

- Thread-local transaction lists; each thread publishes its minimum start ts as an atomic 64-bit; global minimum in O(#threads), no latches. [V 2026-10-25] sect. 4.2
- Eager Pruning of Obsolete versions (EPO): whenever a thread touches a version chain (on update, while it holds the chain lock) it keeps for each active start ts only the version visible to it, merging delta attributes into the kept version; a chain never exceeds the number of active transactions. Active-ts list refreshed about every 5 ms with no measurable cost. [V 2026-10-25] sect. 4.3
- HANA's interval GC differs: dedicated thread every 10 s scans versions committed after the oldest active start. [V 2026-10-25] sect. 4.3.2, Table 2
- Version record: Type 1, Version 4 (commit ts plus lock bit), RelationId 2, Next pointer 4, TupleId 4; delete 15 bytes, update 19 + before-images with a 64-bit attribute mask for < 64 attributes, insert 11 + 8 per tuple id (one record for a bulk insert). Byte split per field inferred from the totals. [V 2026-10-25] Table 3, sect. 4.4
