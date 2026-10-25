# LeanStore

kind: system
home: https://db.in.tum.de/~leis/papers/leanstore.pdf

## B-tree and buffer manager

- Leis et al. ICDE 2018: buffer-managed B+tree with pointer swizzling (child pointers hold either an in-memory pointer or a page id), a cooling stage FIFO (about 10% of pages) for speculative unswizzling, epoch-based eviction instead of pin counters, and optimistic latches (update counter per page) so lookups on swizzled pages take no latches; conflicts restart the operation. 16 KB pages in the evaluation. An inner node is evicted only after all children are unswizzled. [V 2026-10-25] https://db.in.tum.de/~leis/papers/leanstore.pdf

## Swips and cooling

- A swip is an 8-byte reference tagged by one bit: swizzled (pointer) or unswizzled (page id); a hot access is one predicted branch. [V 2026-10-25] https://db.in.tum.de/~leis/papers/leanstore.pdf sec. III
- Cooling stage used only when free pages run low: a random subset (e.g. 10% of the pool) is unswizzled into a FIFO queue plus a hash table by page id; access in cooling re-swizzles; the FIFO tail is flushed and evicted. [V 2026-10-25] same paper sec. IV-C
- Each page has exactly one incoming swip; parent pointers in the buffer frame; children unswizzled before parents. Multiple parents would need "fat" swips (page id plus pointer) as a proposed extension. [V 2026-10-25] same paper sec. IV
- Fig. 1 single-threaded in-memory TPC-C (100 warehouses) compares BerkeleyDB, WiredTiger, LeanStore, in-memory B-tree; LeanStore near in-memory. [V 2026-10-25] same paper
