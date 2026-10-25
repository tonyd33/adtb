---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 640
summary: no simulator driver runs random multi-transaction workloads per seed and checks their histories
branch: adtb-txn
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
