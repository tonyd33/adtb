---
title: Steam garbage collection (Böttcher et al. VLDB 2019)
---
Thread-local transaction lists, a lock-free global minimum, and eager
pruning: every chain touched keeps only versions visible to some active
snapshot, so chain length is bounded by active transactions even under
a long reader. Compares HANA's 10 s interval GC and Hekaton's
background GC. PVLDB 13(2), not SIGMOD.
https://www.vldb.org/pvldb/vol13/p128-bottcher.pdf
