---
created-at: 2026-10-30
status: open
kind: feature
priority: 450
summary: "online verify pins the vacuum horizon for its run, ignores cancel, holds each index in memory and misses a lost write on a resident page"
---

# Feature: Verify online follow ups

## Request

## Proposal

## Open questions

## 2026-10-30: left by the review

- The snapshot pins the vacuum horizon for the whole run, and vacuum and `set setting RetentionWindow` busy-wait through the index phase, over every relation even for `verify r`.
- Cancel and deadline are ignored: `api/command.rs` passes no bounds, and `waits.wait` cannot be cancelled.
- `write/verify.rs` holds each index whole in `entries` and `Held`, outside the query budget.
- `storage/verify.rs`: a resident page's LSN wins over the file's, so a clean page whose write-out was lost goes unseen until eviction and the next run. Compare the file's LSN under the exclusive latch when the page is not changed.
- `sys::jobs` is overwritten whole, which breaks once reindex or compact add rows.
- Dead kept versions build up in the roots heap.
- `verify` in a read-only transaction is now refused, but `adtb verify FILE` still writes to the file on every run: recovery, a commit record, kept rows and a checkpoint at close.
