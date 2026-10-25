---
title: How the own pager caches pages in memory
pick: one
raised-by: [engine/own-pager]
options:
  mmap:
    title: Map the data files and let the OS page cache be the buffer pool
    links:
      - {rel: prior-art, to: prior/buffer-mmap}
  hash-pool:
    title: Own buffer pool of fixed frames behind a page-id hash table
    links:
      - {rel: prior-art, to: prior/buffer-pg}
      - {rel: prior-art, to: prior/buffer-innodb}
  swizzle:
    title: Own buffer pool with pointer swizzling in page references
    links:
      - {rel: prior-art, to: prior/buffer-leis}
  vmcache:
    title: Own buffer pool over anonymous virtual memory indexed by page id
    links:
      - {rel: prior-art, to: prior/buffer-leis}
---
Every heap, union index and catalog page passes through this layer.
Per-constructor heaps mean many small page sets; a union index probe
lands on a heap page named by `(ctor, tid)`, so heap pages are reached
from index entries, not from one parent.

## buffer/cache/mmap

`mmap(MAP_SHARED)` each file, read pages by pointer, the kernel faults
and evicts. LMDB, SQLite's optional mmap, MonetDB. Crotty, Leis, Pavlo
(CIDR 2022): read-only random reads at about half of `O_DIRECT` pread
once the page cache fills, scans over 10 SSDs about 20x slower, I/O
errors as `SIGBUS`, no async faults.
https://db.cs.cmu.edu/papers/2022/cidr2022-p13-crotty.pdf

## buffer/cache/hash-pool

Postgres shared buffers, InnoDB, SQLite pager. A frame array, a
descriptor per frame, and a hash table `(file, page) -> frame`. Every
access, hit or miss, hashes and latches a bucket.

```
frame descriptor (Postgres master, 64-bit state word)
  bits  0..17  refcount (pins)
  bits 18..21  usage count (clock), cap 5
  bits 22..33  flags: dirty, valid, io-in-progress, ...
  bits 34..53  content lock
  + tag (rel, fork, block), frame index
```
https://github.com/postgres/postgres/blob/master/src/include/storage/buf_internals.h

## buffer/cache/swizzle

LeanStore (ICDE 2018), Umbra (CIDR 2020): a reference inside a page is
a swip, either a pointer to the frame or a page id. A hot access is one
bit test, no hash table.

```
swip (u64, Umbra)
  swizzled:    [63..1 pointer | 0]
  unswizzled:  [57-bit page id | 6-bit size class | 1]
```
Each page has exactly one owning swip.
https://www.cidrdb.org/cidr2020/papers/p29-neumann-cidr20.pdf

## buffer/cache/vmcache

Leis et al. (SIGMOD 2023): reserve anonymous virtual memory the size of
storage; page `p` lives at `base + p * 4096`; `pread` into it on a
fault the engine detects, `madvise(MADV_DONTNEED)` to evict. Any page
graph works, pages may span any multiple of 4 KiB.

```
state[p] (u64 per storage page)
  bits 56..63  0 unlocked | 1..252 shared count | 253 locked
               | 254 marked (clock) | 255 evicted
  bits  0..55  version, bumped on unlock and eviction
```
About 16 bytes of DRAM per 4 KiB of storage: 4 GiB for 1 TiB.
https://www.cs.cit.tum.de/fileadmin/w00cfj/dis/_my_direct_uploads/vmcache.pdf
