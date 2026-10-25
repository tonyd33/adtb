---
title: Bw-tree and its reimplementation
---
Lock-free B-tree (Levandoski et al. ICDE 2013): delta records prepended
to nodes, a mapping table from node id to address swapped by CAS,
consolidation of chains. Wang et al. (SIGMOD 2018) found even an
optimised OpenBw-Tree slower than ART, Masstree and a B+tree with
optimistic lock coupling.
https://www.cs.cmu.edu/~huanche1/publications/open_bwtree.pdf
