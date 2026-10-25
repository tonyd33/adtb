---
title: vmcache needs one page-id space and fast unmapping
pick: any
raised-by: [buffer/cache/vmcache]
options:
  global-pid:
    title: One page-id space over all storage, files are extents of it
  exmap:
    title: Use the exmap kernel module for batched unmapping without shootdown storms
    excludes: [buffer/platform/portable]
  batch-evict:
    title: Evict in batches and pay one madvise call and shootdown per batch
  live:
    title: Live with it
    excludes: [buffer/cache/vm-reserve/global-pid, buffer/cache/vm-reserve/exmap, buffer/cache/vm-reserve/batch-evict]
---
`base + pid * 4096` needs pids dense over every heap and index. With a
file per constructor heap (`layout/files/per-heap-file`) pids are
per file. Each `MADV_DONTNEED` shoots down remote TLBs; Leis et al.
found stock Linux page-table manipulation a bottleneck on fast SSDs.

## buffer/cache/vm-reserve/global-pid

A single data file, or an extent map `pid range -> (file, offset)`
consulted on miss only. Reservation is virtual: 1 TiB of storage is
1 TiB of address space and 4 GiB of state plus page tables.

```
virtual: [ catalog | identities.EmailPass extent | identities.Google extent | identity_pk ... ]
extent map (miss path): pid 0x40000..0x7ffff -> file 0, offset 1 GiB
```

## buffer/cache/vm-reserve/exmap

Linux-only module, per-thread free lists of physical pages, batched
TLB invalidation (Leis et al. sec. 4). A kernel module is an install
step for an embedded library.

## buffer/cache/vm-reserve/batch-evict

The vmcache eviction loop: collect marked candidates, write dirty
ones, one `madvise` per batch. Portable (Windows: `VirtualAlloc`
`MEM_RESET`, unverified equivalence).

## buffer/cache/vm-reserve/live

Eviction-heavy workloads stall in the kernel.
