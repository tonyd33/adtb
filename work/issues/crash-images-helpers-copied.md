---
created-at: 2026-10-30
status: in-progress
branch: quality-fixes32
kind: bug
priority: 790
summary: "crash_images.rs repeats the FNV of common.rs, and allocate has no helper in common beside every other op"
tags:
  - quality
  - sim
  - test
---

# crash_images.rs repeats the FNV hash common.rs holds, and allocate has no shared helper

## Report

`crates/adtb-sim/tests/crash_images.rs:16` and `:20` restate the offset, the prime and the loop of `fnv` at `crates/adtb-simkit/src/common.rs:97`, which is private and has no way to continue from a digest.

`crash_images.rs:34` defines `allocate` over `complete`, while `crates/adtb-sim/tests/faults.rs:209` and `:507` build `Op::Allocate` by hand. `common` has a helper for every other op: `open`, `write`, `read`, `sync`, `sync_dir`.

Fix direction: make `common::fnv` a public fold taking the digest so far, keep `record`'s value, and add `common::allocate`.
