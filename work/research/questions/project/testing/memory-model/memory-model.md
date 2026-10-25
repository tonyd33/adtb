---
title: Testing latch and atomic code under the memory model
pick: any
raised-by: [btree/latch/racy/atomic-reads]
options:
  loom:
    title: Exhaustive interleavings of small latch tests under the C11 model
    links:
      - {rel: prior-art, to: prior/project-rust-concurrency-tools}
  miri:
    title: Run the simulated engine under Miri for UB and data races
    links:
      - {rel: prior-art, to: prior/project-rust-concurrency-tools}
  live:
    title: Rely on the simulated scheduler alone
    excludes: [project/testing/memory-model/loom, project/testing/memory-model/miri]
---
Optimistic lock coupling reads pages with atomic loads and validates a
version afterwards. The simulated scheduler interleaves threads at
yield points but runs on real hardware memory ordering, so it does not
explore weak-memory reorderings.

## project/testing/memory-model/loom

```
#[cfg(loom)] #[test]
fn olc_reader_sees_consistent_or_restarts() {
  loom::model(|| { let p = Arc::new(Page::new()); /* writer thread, reader thread */ });
}
```

## project/testing/memory-model/miri

Miri reports out-of-bounds, use-after-free, invalid values, data races
and some weak-memory effects. It cannot run io_uring, so it runs the
engine over the simulated storage.

## project/testing/memory-model/live

Weak-memory bugs found only on hardware that reorders more than x86.
