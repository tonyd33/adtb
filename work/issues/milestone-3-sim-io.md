---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 300
summary: "milestone 3 (simulation and I/O) has not exited: no Loom tests of the page state word, and Miri does not run the adtb-sys tests"
closed: 2026-10-29
depends-on:
  - io-backends
  - real-runtime
  - loom-and-miri
tags:
  - milestone
---

# Feature: Milestone 3, simulation and I/O

## Report

Milestone 3 of `docs/plan.md`. Exit: a seeded run replays byte for
byte; Miri runs the simulator tests clean.

The simulator, the simulated clock, randomness, scheduler and
`SimStorage` are done, and milestone 4 needs nothing more from this
milestone. Open on 2026-10-25: `io-backends.md`, `real-runtime.md`,
`loom-and-miri.md`, `sim-network-and-worker-loop.md`.

## 2026-10-29: exit narrowed

Real backends and runtime are resolved. docs/plan.md now exits milestone 3 on
seeded replay, Loom model tests of the page state word, and Miri over the
adtb-sys tests of the page table and aligned buffers. Every engine crate forbids
unsafe code; the unsafe blocks are in adtb-sys (pages.rs, buf.rs, os/uring.rs, and
io_uring cannot run under Miri), so Miri over the whole simulator suite would check
safe code for hours. The simulated network and worker loop moved to milestone 10,
whose transports are the only thing that needs them.

## 2026-10-29: closed

Exit met: Loom models pass and Miri runs adtb-sys clean (loom-and-miri, landed 0e560be9). Replay byte for byte holds under the suite.

## 2026-10-29: resolved
