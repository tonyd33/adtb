---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 315
summary: "there is no real runtime: no real clock, threads or entropy behind the runtime interfaces"
closed: 2026-10-25
tags:
  - sim
---

# Feature: real runtime

## Report

The runtime interfaces have only the deterministic simulator behind
them. A real runtime needs a clock, threads and entropy. Entropy needs
`getrandom` or `libc` approved in `deny.toml`.

## 2026-10-25: resolved

## 2026-10-25: resolved

`adtb_sys::os::OsRuntime` (Linux) gives `CLOCK_MONOTONIC`, `getrandom` entropy and real threads, with the blocking-pool backend behind `OsIo`. `crates/adtb-sim/tests/io_contract.rs` runs one contract over the simulator and the pool; `real_file.rs` creates, reopens and verifies a database on a real file.
