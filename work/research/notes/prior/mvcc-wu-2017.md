---
title: Wu et al., An Empirical Evaluation of In-Memory MVCC (VLDB 2017)
---
Taxonomy of version storage (append-only O2N or N2O, time-travel,
delta), GC (tuple-level background or cooperative, transaction-level
epochs) and index pointers (logical via indirection or primary key,
physical per version). Version storage matters most for scaling; N2O
beat O2N; transaction-level GC smallest footprint; logical pointers
win with many secondary indexes. Postgres classed append-only O2N,
InnoDB and Oracle delta, HyPer delta, Hekaton O2N.
https://www.vldb.org/pvldb/vol10/p781-Wu.pdf
