---
created-at: 2026-10-30
status: in-progress
branch: quality-fixes33
kind: bug
priority: 665
summary: "BTree::bulk checks no stop, so a close can wait out one bulk write of a reindex"
---

# Reindex bulk no stop

## Report

`merge_runs` has no stop check either.
