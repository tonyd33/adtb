---
created-at: 2026-10-25
status: open
kind: feature
priority: 550
summary: "the heap copies whole pages per change, decodes large values to compare keys, and recomputes record offsets on every encode"
depends-on:
  - slotted-heap
tags:
  - engine
---

# Feature: Heap write costs

## Report

Found while building the heap:
- staging copies every page it touches;
- `update` decodes the old row, large values included, to compare its
  key fields;
- record offsets are recomputed on every encode.

Measure each one with a benchmark under the simulator before changing
it.
