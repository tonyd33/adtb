---
created-at: 2026-10-25
status: wontfix
kind: feature
priority: 270
summary: there is no harness that drives an application on adtb under simulated high load and checks the results
closed: 2026-10-25
depends-on:
  - prealpha-embedding-api
tags:
  - prealpha
  - test
---

# Feature: Load harness for the pre-alpha

## Report

A harness that runs an application's transactions from many simulated clients with a seeded generator, records each commit, and checks the final state against a replay of the committed history on the reference interpreter, plus constraint checks. Throughput numbers from the pre-alpha measure the interpreter, not the engine; the point is correctness and API ergonomics under contention (retries, serialization).

## 2026-10-25: wontfix

Dropped: Tony decided the first application waits until the engine is ready, so no interpreter-backed pre-alpha is built.
