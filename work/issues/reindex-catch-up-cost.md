---
created-at: 2026-10-30
status: open
kind: bug
priority: 660
summary: "the last reindex catch-up holds the index latch while it reads every page of every heap of the set, so writers stall in proportion to heap size"
---

# Reindex catch up cost

## Report

`mend_history` also runs under the index latch: every rebuild of a key index reads the whole history tree with it held, damage or not, and a tree with unmarked damage stalls writers for a read of every history heap. Move the judgement before the latch.
