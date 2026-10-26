---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 557
summary: "some heap updates latch more than the per-thread bound of 5, which the pager does not enforce"
branch: adtb-heapg
tags:
  - engine
---

# Bug: Heap updates past the per-thread latch bound

## Report

`docs/design/storage.md` §2.3 bounds a thread at 5 latches, and pool
sizing relies on it, but the pager does not enforce it. Two heap paths
exceed it:
- a main update with two heap pages, the new version's store heads, a
  directory path on a raise, and two visibility map leaves;
- `large.write` during a conversion while heap pages are held.

Expected: under the simulator and in debug builds, the pager panics
when a thread holds more than the bound, and both paths are split into
updates that compose crash-safely within it.
