---
created-at: 2026-10-30
status: resolved
kind: bug
priority: 620
summary: "no test states what a crash leaves of an allocate: crash_images pins digests only, and its header omits that the trace is folded in"
closed: 2026-10-30
tags:
  - quality
  - sim
  - test
---

# After a re-bless of crash_images nothing states what a crash leaves of an allocate

## Report

`crates/adtb-sim/tests/crash_images.rs` is the only direct test of `Op::Allocate` across a crash, and it is nine FNV digests. A digest shows the bytes match the code that blessed it, never that they are right. The next legitimate change of a draw or a trace line replaces every literal, and from then on a wrong image passes.

The other direct allocate tests stop before a crash: `crates/adtb-sim/tests/faults.rs:497` and `crates/adtb-sim/tests/io_contract.rs:120` read the visible file only. Engine tests reach it only through the log's segments.

The header at `crash_images.rs:1` says a difference means "the simulator's bytes or its random draws changed". `crash_digest` (`:83`) and the segment case (`:274`) also fold `sim.trace()`, so rewording one `trace!` line anywhere in the simulator fails the file with no hint why.

Unpinned by any assertion:

- A forced allocate survives a crash at its full length, all zeros.
- An unforced allocate leaves the old length or more, never less, and every byte past the old durable contents is zero, torn included.
- A forced write inside an allocated range keeps its bytes when the allocate is torn or lost beneath it.

Fix direction: add a seed sweep asserting those three directly on `sim.files()`, with the seed in each message. Then either drop the trace from the digests or correct the header.

## 2026-10-30: from the sim-disk review

No digest in crash_images.rs covers a crash during an in-flight op: `after` is always 10 s, so every op has completed. `Payload::Bytes(w.data)` in the crash path of disk.rs is unpinned. All digest lengths are multiples of `atomic_unit`, so the partial last unit is unpinned too. Add short `after` values and a length off the unit.

## 2026-10-30

Fixed in the quality-fixes32 commit; check passed with 3023 tests.

## 2026-10-30: resolved
