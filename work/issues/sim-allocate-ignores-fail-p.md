---
created-at: 2026-10-30
status: resolved
kind: bug
priority: 610
summary: "Disk::allocate draws no fail_p while the faulty real backend fails allocate under it, so a simulator run never takes that path"
closed: 2026-10-30
tags:
  - quality
  - sim
  - test
---

# The simulator never fails an allocate under fail_p, and the faulty real backend does

## Report

`crates/adtb-sys/src/os/faults.rs:159` fails `Op::Allocate` with `IoError::Failed` under `fail_p`, and `crates/adtb-sim/tests/io_contract/faults.rs:52` tests that. `Disk::allocate` at `crates/adtb-sim/src/disk.rs:505` draws nothing and can only return `Full`, and `Faults::fail_p` documents "a write or force".

So a simulator run never takes the path where `allocate` fails with `Failed` (`crates/adtb/src/storage/driver.rs:104`, fatal). The "failing" case of `crash_images.rs` runs five allocates at `fail_p` 0.3 and none can fail.

Fix direction: draw `fail_p` in `Disk::allocate`, as `write` and `sync` do, and state it in the field's doc. This changes the draws, so the digests in `crash_images.rs` move with it. Run the crash sweeps at a nonzero `fail_p` to see the fatal path hold.

## 2026-10-30

Fixed in the quality-fixes32 commit; check passed with 3023 tests.

## 2026-10-30: resolved
