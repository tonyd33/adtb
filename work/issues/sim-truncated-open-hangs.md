---
created-at: 2026-10-30
status: resolved
kind: bug
priority: 427
summary: "a simulated file truncated before the first roles page: open did not return within 60 s; real files not tried, cause not diagnosed"
closed: 2026-10-30
---

# Sim truncated open hangs

## Report

## 2026-10-30: resolved

Engine fault, not the simulator: a page past the end reads as zeros, and `Heap::put_version` looped on it. Open now returns on every truncation tried. Pages a set bitmap bit names past the end are refused after recovery, and a named page reading all zero is damage on every read. Two `disallowed_methods` expects stay, for the real-thread bound on a hang. Branch check exit 0, 3417 tests. Left: truncated-to-nothing, lost-bitmap-reuse.

## 2026-10-30: resolved
