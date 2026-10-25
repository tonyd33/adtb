---
title: Miri cannot execute io_uring or direct I/O
pick: any
raised-by: [project/arch/unsafe/confined, project/arch/unsafe/audited]
options:
  sim-storage:
    title: Run the engine under Miri over the simulated storage
    needs: [buffer/test/dst]
  sys-tests:
    title: Test the unsafe system module natively with sanitizers
  live:
    title: Leave the system layer unchecked by Miri
    excludes: [project/arch/unsafe/miri-io/sim-storage, project/arch/unsafe/miri-io/sys-tests]
---
Miri interprets Rust and supports only some foreign calls (unverified
for io_uring; mmap support partial, unverified).

## project/arch/unsafe/miri-io/sim-storage

The DST storage double is plain Rust, so the pager, latches and page
layouts run under Miri with simulated I/O; only the real backend stays
outside.

```
cargo miri test -p adtb --features sim
```

## project/arch/unsafe/miri-io/sys-tests

AddressSanitizer and ThreadSanitizer on the native `sys` module.

## project/arch/unsafe/miri-io/live

The I/O backend's unsafe is trusted.
