---
created-at: 2026-10-25
status: open
kind: feature
priority: 540
summary: "the B-link tree has no posting-list deduplication, kill bits, empty-page recycling or right-edge append"
depends-on:
  - blink-tree
tags:
  - engine
---

# Feature: B-link space reuse

## Report

`docs/design/indexes.md` §8.1 lists the following, and none is built:
- posting-list deduplication;
- kill bits, and cleanup before a split;
- unlinking and recycling empty pages;
- the right-edge append for sequential keys.

Deleted cells also stay as garbage until an insert compacts the page.
Kill bits need the unlogged change still undecided in
`docs/design/storage.md` §2.3.
