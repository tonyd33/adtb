---
created-at: 2026-10-26
status: open
kind: bug
priority: 599
summary: vacuum_threads crashes_beside_concurrent_vacuum_recover_clean panics at seed 13 with Xid(0) is not a reserved id
tags:
  - sim
  - engine
---

# Bug: vacuum crash recovery panics on an unreserved xid

## Report

`crates/adtb-sim/tests/vacuum_threads.rs` `crashes_beside_concurrent_vacuum_recover_clean` panics at seed 13 with `Xid(0) is not a reserved id`, in the assert at `crates/adtb/src/mvcc/clog.rs:394` inside `status`. The panicking thread is "other 1". Only the deep profile reaches seed 13; the default profile's sweep does not.

```
cargo nextest run --profile deep -p adtb-sim --test vacuum_threads
```

Reproduced on workspace rqxztvot with `SEEDS = 13..14`.