---
created-at: 2026-10-25
status: open
kind: feature
priority: 640
summary: no simulator driver runs random multi-transaction workloads per seed and checks their histories
depends-on:
  - ssi
  - isolation-checker
tags:
  - test
  - sim
---

# Feature: A random-workload isolation driver

## Report

Milestone 6 exits when no G0, G1 or G2 cycle appears over a fixed number
of seeds per run. `adtb-isolation` checks histories and runs fixed
scenarios, but nothing generates random list-append workloads over many
simulated clients with crashes, and checks each seed's history. Build
it over the engine's `Database` implementation from `ssi.md`, with the
seed count fixed per run.

## 2026-10-25: resolved

Landed as `feat(mvcc): run the transaction layer without a global latch and check random workloads for anomalies`. `crates/adtb-sim/tests/isolation.rs` runs 32 seeds of random clients with crashes and finds no anomaly. A transaction is acknowledged only once the log is durable past every commit it read (`Transactions::read_upto`).

## 2026-10-25: a blind spot

The driver passes even when read-only commits ignore `read_upto`: a
read-write commit flushes at once, so a reader rarely sees an unflushed
commit before a crash. A unit test covers the rule, but the driver
should catch it too. Delay commit flushes in some seeds, so a reader
can see a commit that is not yet durable.
