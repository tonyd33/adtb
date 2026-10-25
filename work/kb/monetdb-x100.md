# Boncz, Zukowski, Nes, "MonetDB/X100: Hyper-Pipelining Query Execution" (CIDR 2005)

kind: paper
home: https://www.cidrdb.org/cidr2005/papers/P19.pdf

## Execution

- Volcano-style pull of vectors (default 1024 values) of cache-resident vertical chunks; work done in vectorized primitives (`map_*`, `select_*`) generated from patterns; hundreds of primitives. [V 2026-10-25] sec. 4
- Every primitive takes an optional selection vector of selected positions; leaving child vectors intact after a selection is often quicker than copying. [V 2026-10-25] sec. 4.2
- Compound primitives (fused expressions) often twice as fast as single-function primitives. [V 2026-10-25] sec. 4.2
- MySQL on TPC-H Q1: 38 instructions per addition at IPC 0.8, about 49 cycles; less than 10% of time in the actual arithmetic. [V 2026-10-25] sec. 3.1, Table 2
- TPC-H Q1 SF1 (s, AthlonMP): MySQL 4.1 26.6, DBMS X 28.1, MonetDB/MIL 3.7, X100 0.50, hand-coded 0.22. [V 2026-10-25] Table 1
- Vector size: 1 suffers interpretation overhead like MySQL; 128 to 8K work well; beyond 8K the vectors exceed the 320 KB L1+L2 of the AthlonMP. [V 2026-10-25] sec. 5.2, Fig. 10
