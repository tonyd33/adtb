# Crotty, Leis, Pavlo: Are You Sure You Want to Use MMAP in Your DBMS?

kind: paper
home: https://db.cs.cmu.edu/papers/2022/cidr2022-p13-crotty.pdf

## Claims

- CIDR 2022. Four problems with file-backed mmap as a buffer pool: transactional safety (OS may flush a dirty page any time, before commit), I/O stalls (no async reads; any access may fault), error handling (checksums must be checked on every access; I/O errors arrive as SIGBUS anywhere), performance (page table contention, single-threaded kswapd eviction, TLB shootdowns). [V 2026-10-25] https://db.cs.cmu.edu/papers/2022/cidr2022-p13-crotty.pdf
- mlock does not stop the OS writing a dirty pinned page back, per POSIX and Linux. MADV_NORMAL faults in the page plus next 16 and previous 15 (128 KB at 4 KB pages). [V 2026-10-25] same paper sec. 2.2
- Update safety schemes classified: OS copy-on-write (MAP_PRIVATE workspace plus WAL; MongoDB MMAPv1 only), user-space copy-on-write (SQLite, MonetDB, RavenDB), shadow paging (LMDB, single writer, msync then pointer switch). [V 2026-10-25] same paper sec. 3.1
- Table 1 mmap users: MonetDB, MongoDB (2009-2019), LevelDB, LMDB, SQLite (option), SingleStore (2013-2015), QuestDB, RavenDB, InfluxDB (2015-2020), WiredTiger (2020-, limited). RocksDB forked LevelDB partly over mmap read bottlenecks; SingleStore saw 10-20 ms per query in mmap lock contention. [V 2026-10-25] same paper sec. 2.3
- Experiment: AMD EPYC 7713, 100 GB page cache, PM1733 SSDs, Linux 5.11. Random reads over 2 TB with 100 threads: fio O_DIRECT about 900K reads/s; mmap MADV_RANDOM matched for 27 s, dropped near zero for about 5 s when the cache filled, then about half of fio. Sequential scan over 10 SSDs: about 20x gap. Summary 2-20x worse once eviction starts. [V 2026-10-25] same paper sec. 4
- Conclusion: maybe use mmap only when the working set fits in memory and the workload is read-only. Points to pointer swizzling (LeanStore, Umbra) as the lightweight alternative. [V 2026-10-25] same paper sec. 6
