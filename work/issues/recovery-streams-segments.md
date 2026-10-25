---
created-at: 2026-10-25
status: open
kind: feature
priority: 490
summary: "recovery and verify read whole log segments into memory"
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
