---
created-at: 2026-11-05
status: open
kind: bug
priority: 620
summary: "a page found Stale after its backup copy, by evidence the lost-write check cannot see, is no longer listed in the manifest, so the proof passes"
---

# Backup manifest misses a page found stale after its copy

## Report

## Report

After `fix: spoil a backup's copy of a page whose write was lost`, the manifest's damaged list holds exactly the pages copied spoiled; `copy_pages` no longer adds the pager's damage map at the end. A page refused `Stale` only after its copy, by evidence `Pager::lost_write` cannot see at copy time (a log-chain break `verify` finds after the page's evicted-table slot was taken by another eviction), was listed on main and is now unlisted, so `test_restore` and the proof pass over a copy that may hold a lost write.

Found in review of the lost-write track, 2026-11-05; reasoned, not reproduced.

## Proposal

Decide whether late damage belongs in the manifest under a separate name (pages refused during the backup whose copy was not spoiled), keeping §7.1's list meaning exact; or widen the evicted table so the evidence survives. Test with a forced slot collision.
