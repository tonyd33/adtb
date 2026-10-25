# Leis et al.: Virtual-Memory Assisted Buffer Management (vmcache, exmap)

kind: paper
home: https://www.cs.cit.tum.de/fileadmin/w00cfj/dis/_my_direct_uploads/vmcache.pdf

## Design

- SIGMOD 2023 (Leis, Alhomssi, Ziegler, Loeck, Dietrich). Reserve anonymous virtual memory the size of storage; page id = offset; fault in with pread into virtMem+offset; evict with madvise(MADV_DONTNEED). DBMS keeps control of faulting and eviction; TLB does translation. [V 2026-10-25] https://www.cs.cit.tum.de/fileadmin/w00cfj/dis/_my_direct_uploads/vmcache.pdf
- Page state array, one 64-bit word per storage page: 8 bits state (Unlocked 0, LockedShared 1-252, Locked 253, Marked 254, Evicted 255) plus 56-bit version for optimistic reads. Version bumps on unlock and on eviction. [V 2026-10-25] same paper sec. 3
- DRAM overhead about 16 bytes per 4 KB of storage (8.016 for 5-level page tables, 8 for state); 1 TB SSD needs about 4 GB. [V 2026-10-25] same paper sec. 3.6
- Clock replacement: Unlocked pages set to Marked; any access clears the mark; resident pages tracked in a fixed-size open-addressing hash table used only for eviction, not on hits. Eviction batches: lock marked candidates, write dirty, madvise, drop. [V 2026-10-25] same paper sec. 3
- Any page size that is a multiple of 4 KB; state of the head page governs; virtual contiguity without physical contiguity. Umbra by contrast only powers of two and still swizzles. [V 2026-10-25] same paper sec. 3.5, Table 1
- Supports arbitrary graphs of page references, which pointer swizzling does not (one owning parent per page). [V 2026-10-25] same paper sec. 2
- exmap: Linux kernel module for scalable page-table manipulation, fixing madvise/TLB-shootdown cost on fast SSDs. [V 2026-10-25] same paper sec. 4
- Recommends 4 KB pages for OLTP; 4 KB or 2 MB for OLAP. [V 2026-10-25] same paper sec. 3.5
