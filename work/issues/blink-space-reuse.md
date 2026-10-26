---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 540
summary: "the B-link tree has no posting-list deduplication, kill bits, empty-page recycling or right-edge append"
closed: 2026-10-25
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

## 2026-10-25: leaf merges

Leaves are never merged, so scans can walk near-empty leaves. A
`put` that passes a split by a right link while carrying a separator up
leaves it for the next insert or repair to finish.

## 2026-10-25: resolved

Landed as `feat(btree): unlink emptied leaves by fence keys, keep postings and clean up before splitting`. Pages carry fence keys; an emptied leaf is unlinked and freed in one batch. Leaf merges stay ruled out by `btree/merge/empty-only`. Follow-ups are in `blink-follow-ups.md`.
