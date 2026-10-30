---
created-at: 2026-10-30
status: open
kind: bug
priority: 700
summary: explain still names a marked key index that the heaps serve
---

# Quarantine explain index

## Report

A key lookup on a marked key index names that index in `explain`, though the store answers the probe from the heaps.
