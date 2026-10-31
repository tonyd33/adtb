---
created-at: 2026-10-30
status: resolved
kind: bug
priority: 665
summary: "BTree::bulk checks no stop, so a close can wait out one bulk write of a reindex"
closed: 2026-10-30
---

# Reindex bulk no stop

## Report

`merge_runs` has no stop check either.

## 2026-10-30

Landed on main. Check passed, 3310 tests.

## 2026-10-30: resolved
