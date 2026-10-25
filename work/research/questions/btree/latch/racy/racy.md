---
title: Optimistic readers read page bytes a writer is changing
pick: any
raised-by: [btree/latch/olc]
options:
  atomic-reads:
    title: Page reads go through relaxed atomic or volatile loads, validated before use
  epoch-evict:
    title: The buffer manager evicts a page only after every reader epoch has passed
    links:
      - {rel: prior-art, to: prior/btree-art-olc}
  live:
    title: Live with reads that are data races in the language memory model
    excludes: [btree/latch/racy/atomic-reads, btree/latch/racy/epoch-evict]
    assessment: {grade: broken, why: a plain racing read is undefined behaviour in Rust}
---
The engine is Rust. Under OLC a reader copies keys while a writer
shifts them; validation discards the result, but the read itself is a
race. A page may also be evicted and its frame reused mid-read.

## btree/latch/racy/atomic-reads

Read through `AtomicU64::load(Relaxed)` or `read_volatile` into local
copies; never form `&[u8]` over a page another thread may write.
Lengths and offsets read optimistically are bounds-checked before use.

## btree/latch/racy/epoch-evict

LeanStore: threads enter a global epoch on each operation; a frame is
reused only after all threads have left the epoch in which it was
unswizzled.

## btree/latch/racy/live

Accept.
