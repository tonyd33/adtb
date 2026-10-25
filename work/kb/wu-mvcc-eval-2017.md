# Wu, Arulraj, Lin, Xian, Pavlo: An Empirical Evaluation of In-Memory MVCC

kind: paper
home: https://www.vldb.org/pvldb/vol10/p781-Wu.pdf

## Version storage

- Three schemes: append-only (every version a full tuple in the table), time-travel (master version in main table, older copies in a separate time-travel table), delta (master in main table updated in place, old values of modified attributes only in delta storage, "rollback segment" in MySQL and Oracle, also HyPer). [V 2026-10-25] VLDB 10(7) 2017 sect. 4
- Version chain is a singly linked latch-free list through each version's pointer field; HEAD is newest (N2O) or oldest (O2N). O2N: indexes need no update per new version but readers traverse to the latest; N2O: indexes must be repointed at every update unless an indirection layer is used. [V 2026-10-25] sect. 4.1
- Time-travel: indexes always point at the master version, unaffected by chain updates. SQL Server keeps the current version as master; SAP HANA keeps the oldest as master. [V 2026-10-25] sect. 4.2
- Delta: reading several attributes of an old version walks the chain per attribute; low table-scan performance. [V 2026-10-25] sect. 4.3, 8
- Non-inline attributes (BLOBs) copied per version in append-only and time-travel unless versions share refcounted out-of-line data. [V 2026-10-25] sect. 4.1
- Table 1: Oracle MV2PL/Delta/VAC/logical(TupleId); Postgres MV2PL+SSI/append-only O2N/VAC/physical; InnoDB MV2PL/Delta/VAC/logical(PKey); HYRISE MVOCC/N2O/none/physical; Hekaton MVOCC/O2N/COOP/physical; MemSQL MVOCC/N2O/VAC/physical; SAP HANA MV2PL/time-travel/hybrid/logical(TupleId); NuoDB MV2PL/N2O/VAC/logical(PKey); HyPer MVOCC/delta/transaction-level/logical(TupleId). [V 2026-10-25] Table 1

## Tuple header

- Each version: txn-id (write lock, 64-bit so one CAS sets it), begin-ts, end-ts, pointer to neighbouring version; MVTO adds read-ts; MV2PL packs txn-id and read-cnt into one 64-bit word. [V 2026-10-25] sect. 2.2, 3

## Garbage collection

- Steps: detect expired versions, unlink from chains and indexes, reclaim space. Expired = created by aborted txn or end-ts below every active txn's id. [V 2026-10-25] sect. 5
- Tuple-level background vacuuming (VAC): periodic scan; optimisations are a latch-free list of invalidated versions and a dirty-block bitmap. Cooperative cleaning (COOP): readers traversing chains record expired versions; works only with O2N; "dusty corners" (Hekaton) need a periodic full pass. [V 2026-10-25] sect. 5.1
- Transaction-level GC: epochs with membership counters; versions written by an epoch's transactions are reclaimed when it and all earlier epochs have no active transactions; needs per-epoch write sets. Peloton experiments use 40 ms epochs. [V 2026-10-25] sect. 5.2, 7
- Long-running transactions stop reclamation of every version generated during their lifetime. [V 2026-10-25] sect. 5.3
- Finding: transaction-level GC gave best performance with smallest memory footprint; GC causes throughput and memory oscillation. [V 2026-10-25] sect. 8

## Index management

- Index entries carry no version information; false positives possible, no false negatives. [V 2026-10-25] sect. 6
- Logical pointers: fixed id (primary key, or 64-bit TupleId through a latch-free hash map to chain HEAD); secondary indexes updated only when indexed attributes change. Physical pointers: entry per version, only possible with append-only; every update inserts into every secondary index (MemSQL, Hekaton). [V 2026-10-25] sect. 6.1-6.2
- Index-only scans impossible unless version info is embedded in index entries; NuoDB stores header metadata apart from tuple data. [V 2026-10-25] sect. 6.3
- Logical pointers 45% higher throughput than physical under high contention with 20 secondary indexes. [V 2026-10-25] sect. 7.5

## Findings

- Version storage matters more than CC protocol for multicore scaling; N2O beats O2N 2.4-3.4x at the highest contention (theta 0.9) despite index repointing; Postgres and Hekaton configurations (O2N) worst on TPC-C; delta worst latency. MVTO works well across workloads. [V 2026-10-25] sect. 7-8
