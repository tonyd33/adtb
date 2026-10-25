---
title: HyPer data-centric compilation and morsel-driven parallelism
---
Neumann, VLDB 2011: produce/consume code generation fuses each
pipeline into one loop pushing tuples in registers between pipeline
breakers; LLVM chain over precompiled C++.
https://www.vldb.org/pvldb/vol4/p539-neumann.pdf
Leis et al., SIGMOD 2014: a dispatcher hands pipeline jobs and morsels
(about 100,000 tuples) to pinned threads, work stealing, NUMA-local.
https://db.in.tum.de/~leis/papers/morsels.pdf
