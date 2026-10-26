---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 490
summary: recovery and verify read whole log segments into memory
closed: 2026-10-25
depends-on:
  - pager-and-log
tags:
  - engine
---

# Feature: Recovery reads the log as a stream

## Report

Recovery and `verify` read each log segment into memory whole. Memory
then grows with the segment size, not with one block. Expected: both
read block by block, and a test bounds their memory under the
simulator.

## 2026-10-25: resolved

Landed as `feat(storage): persist quarantine marks and read the log a chunk at a time`.
