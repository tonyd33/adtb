---
created-at: 2026-10-25
status: open
kind: feature
priority: 318
summary: "no stress tests run the latch protocol and the B-link tree on real OS threads"
depends-on:
  - real-runtime
tags:
  - test
---

# Feature: Real-thread stress tests

## Report

Every concurrency test runs under the simulator's scheduler. Once
`real-runtime.md` lands, run the latch, B-link, SSI and isolation tests
on real OS threads too, with a time budget, as a separate nextest
profile.
