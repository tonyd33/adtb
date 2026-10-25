---
title: Where unsafe code is allowed
pick: one
raised-by: [substrate/new-engine]
options:
  forbid:
    title: "#![forbid(unsafe_code)] everywhere; unsafe only inside dependencies"
    assessment: {grade: costly, why: "io_uring, direct I/O alignment and the vmcache reservation need unsafe somewhere, so a dependency must own it"}
  confined:
    title: Unsafe only in one small module or crate with a stated contract; forbidden elsewhere
  audited:
    title: Unsafe anywhere it pays, each block with a safety comment and tests under Miri
---
Settled mechanisms that touch raw memory: a virtual-memory page table
(`buffer/cache/vmcache`), direct I/O with aligned buffers
(`buffer/io-path/direct`), io_uring submissions whose buffers must
outlive the operation, optimistic page reads by atomics
(`btree/latch/racy/atomic-reads`), byte-level page and key layouts.

## project/arch/unsafe/forbid

tokio-uring passes buffers by ownership so its API is safe; rustix
wraps syscalls. The engine forbids unsafe and inherits what they do.

## project/arch/unsafe/confined

```
adtb::sys::{mem, uring, page}   // every unsafe block lives here
#![deny(unsafe_code)]            // in every other module, allowed in sys only
```

The rest of the engine sees safe types: `Frame`, `PageRef`,
`AlignedBuf`.

## project/arch/unsafe/audited

Unsafe where profiling says so (key comparison, tuple decoding), each
with `// SAFETY:` and coverage under Miri.
