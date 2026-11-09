---
created-at: 2026-11-09
status: open
kind: bug
priority: 310
summary: "HeapWrite::commit and commit_pruned settle the directory after the page update is durable and before releasing large-value counts, so a settle that fails on damage leaves those counts raised and the values never freed"
tags:
  - quality
  - engine
  - class-not-atomic
---

# Bug: A heap write that fails to settle its directory leaves large-value counts raised

## Report

`HeapWrite::commit` (crates/adtb/src/heap/write.rs:138) commits the stage (:161), then calls `above.settle(pager)?` (:162), then releases each pointer of `release` (:166-171). `commit_pruned` (:226) does the same: `stage.commit_with` (:262), `above.settle(pager)?` (:263), then the release of `released` (:267). `Above::settle` (crates/adtb/src/heap/dir.rs:1283) walks and repairs directory pages and fails with a `PageError` on a damaged one. When it fails, the version or the pruned page is already durable and the release loop never runs:

- in `commit`, the counts of `c.dropped` stay one higher than the versions naming them;
- in `commit_pruned`, the counts of the values the freed versions pointed at stay raised, and since those versions are gone no later prune or vacuum releases them.

Either way the store's sweep never frees those values, and verify's chunk check reports each as a head counting more versions than point at it. The caller also receives an error for a page change that is durable. Damage only: it needs a directory page refused during the settle.

This is distinct from large-retain-before-map-refusal, which covers retains committed before a refusal ahead of the page update; here the failure comes after it.

Reproduction, reasoned and not run: plant damage on an inner directory page of a heap whose page holds a version with an out-of-line value that a full-page prune frees, run the prune (through vacuum or an insert pruning on access), and read the value's count after the refusal.

Fix direction: release before settling (the counts change in updates of their own already), or make a failed settle leave the releases to a retry that cannot be skipped, so no error path after the page update leaves counts raised.
