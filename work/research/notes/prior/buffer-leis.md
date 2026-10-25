---
title: Lightweight buffer managers (LeanStore, Umbra, vmcache)
---
LeanStore (ICDE 2018): swips, 8-byte references tagged by one bit as
pointer or page id; one owning swip per page; a cooling FIFO of about
10% of the pool replaces LRU; optimistic version latches.
Umbra (CIDR 2020): swip of 1 tag bit, 57-bit page id, 6-bit size
class; power-of-two page sizes from 64 KiB, each class its own virtual
range, freed with MADV_DONTNEED.
vmcache (SIGMOD 2023): anonymous virtual memory the size of storage,
page id = offset, a 64-bit state word per page (8-bit lock state, 56-bit
version), clock via a Marked state, any multiple of 4 KB; works for
arbitrary page graphs. exmap is its Linux kernel module for scalable
unmapping. About 16 bytes of DRAM per 4 KB of storage.
https://db.in.tum.de/~leis/papers/leanstore.pdf
https://www.cidrdb.org/cidr2020/papers/p29-neumann-cidr20.pdf
https://www.cs.cit.tum.de/fileadmin/w00cfj/dis/_my_direct_uploads/vmcache.pdf
