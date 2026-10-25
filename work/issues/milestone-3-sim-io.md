---
created-at: 2026-10-25
status: open
kind: feature
priority: 300
summary: "milestone 3 (simulation and I/O) has not exited: no real backends, network, worker loop, Loom or Miri"
depends-on:
  - io-backends
  - real-runtime
  - loom-and-miri
  - sim-network-and-worker-loop
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
