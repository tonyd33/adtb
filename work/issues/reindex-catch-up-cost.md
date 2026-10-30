---
created-at: 2026-10-30
status: open
kind: bug
priority: 660
summary: "the last reindex catch-up holds the index latch while it reads every page of every heap of the set, so writers stall in proportion to heap size"
---

# Reindex catch up cost

## Report
