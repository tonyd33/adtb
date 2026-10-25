# Umbra

kind: paper
home: https://www.cidrdb.org/cidr2020/papers/p29-neumann-cidr20.pdf

## Storage

- Neumann, Freitag, "Umbra: A Disk-Based System with In-Memory Performance", CIDR 2020. Variable-size pages in size classes: smallest 64 KiB, each class twice the previous, up to the buffer pool size; one buffer pool shared by all classes. [V 2026-10-25] https://www.cidrdb.org/cidr2020/papers/p29-neumann-cidr20.pdf
- Each size class reserves its own virtual address range; pages are read with pread into it and released with madvise(MADV_DONTNEED), so physical memory does not fragment. [V 2026-10-25] same paper
- Pointer swizzling: a swip holds either a pointer or (page id, size class); each page has exactly one owning swip, so B+-tree leaves have no sibling links. [V 2026-10-25] same paper
- Relations are B+-trees keyed by synthetic 8-byte monotonically increasing tuple ids; nodes are filled, not split, on insert; inner nodes 64 KiB (fanout 8192); a leaf takes the smallest page size holding the tuple. Leaf tuples use PAX: fixed-size attributes columnar at the page start, variable-size data packed at the end. [V 2026-10-25] same paper
- ARIES recovery; disk space is reused only for pages of the same size, since a smaller page written over a larger one's space could expose the old page's bytes as an LSN at recovery. [V 2026-10-25] same paper
- Strings: 16-byte header, 4-byte length; up to 12 bytes stored inline; longer ones keep a 4-byte prefix plus an 8-byte offset (in-page) or pointer, two bits of which encode storage class (persistent, transient, temporary). Long strings are never split across pages. [V 2026-10-25] same paper

## Execution

- Physical plans are modular state machines: pipelines split into steps, single- or multi-threaded; a parallel step function processes one morsel per invocation, so execution can suspend between invocations. [V 2026-10-25] sec. 3.3
- Code is generated into a custom lightweight IR resembling a subset of LLVM IR; each step starts as bytecode interpreted by a VM; for parallel steps progress tracking decides whether to translate to LLVM IR and JIT compile. [V 2026-10-25] sec. 3.3
- Geometric mean speedup over HyPer 3.0x on JOB and 1.8x on TPC-H, partly from adaptive compilation; HyPer spends up to 29x more time compiling than executing on cheap queries. [V 2026-10-25] sec. 4

## Statistics

- Online reservoir sampling keeps an up-to-date sample; updateable HyperLogLog sketches per column give near-perfect single-column distinct estimates; multi-column estimates combine sketches and samples. [V 2026-10-25] sec. 3.2

## Swip layout

- Swip is one 64-bit word: swizzled = pointer with tag bit 0; unswizzled = tag bit 1, 57-bit page id, 6-bit size class. [V 2026-10-25] https://www.cidrdb.org/cidr2020/papers/p29-neumann-cidr20.pdf Fig. 2
- Reads after MADV_DONTNEED see zero bytes (shared zero page), so optimistic readers of an evicted frame never fault on invalid memory. [V 2026-10-25] same paper
