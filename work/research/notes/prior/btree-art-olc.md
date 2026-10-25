---
title: Adaptive radix tree and optimistic lock coupling
---
ART (Leis et al. ICDE 2013): Node4/16/48/256 of 52/160/656/2064 bytes,
lazy expansion and path compression, 52 bytes per key worst case;
needs binary-comparable keys, no key a prefix of another. Optimistic
lock coupling (Leis et al. DaMoN 2016): one 64-bit word per node holds
lock, version and obsolete bit; readers validate versions and restart.
LeanStore (ICDE 2018) applies it to a buffer-managed B+tree. OLFIT
(Cha et al. VLDB 2001) is the earlier version-validated B-tree.
https://db.in.tum.de/~leis/papers/artsync.pdf
