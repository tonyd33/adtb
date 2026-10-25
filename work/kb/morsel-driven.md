# Leis, Boncz, Kemper, Neumann, "Morsel-Driven Parallelism" (SIGMOD 2014)

kind: paper
home: https://db.in.tum.de/~leis/papers/morsels.pdf

## Scheduling

- A dispatcher assigns tasks (pipeline jobs) and morsels (input fragments, e.g. 100,000 tuples) to worker threads pinned to cores; each executes a morsel through the pipeline and materializes into the next pipeline breaker. [V 2026-10-25] sec. 1-3
- Run-time task distribution with work stealing; fully elastic: the number of threads on a query can change at any time. Contrasts with Volcano plan-driven parallelism via exchange operators. [V 2026-10-25] sec. 1
- NUMA-aware: threads read NUMA-local morsels and write NUMA-local storage areas; the hash table for the join is interleaved across sockets. [V 2026-10-25] sec. 3-4
- Morsel size should be the smallest with negligible overhead; above 10,000 in their experiment; about 100,000 used. [V 2026-10-25] sec. 3.4, Fig. 6
- Hash join build: phase 1 materializes morsel-wise NUMA-locally, phase 2 inserts pointers into a lock-free hash table with a 16-bit tag in the pointer for early filtering. [V 2026-10-25] sec. 4.2, Fig. 7
- Average speedup over 30 with 32 cores on TPC-H and SSB. [V 2026-10-25] abstract
