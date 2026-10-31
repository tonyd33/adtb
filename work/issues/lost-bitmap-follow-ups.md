---
created-at: 2026-10-31
status: open
kind: bug
priority: 472
summary: "lost-bitmap tests prove less than they say about bytes kept, and the sweep misses one crash shape"
---

# Lost bitmap follow ups

## Report

From the delta review of lost-bitmap. No loss known; the tests prove less than they should.

- crates/adtb-sim/tests/lost_bitmap.rs `Kept`: on an exempt page it compares only kind and owner. Tighten, same walk: a heap slot normal before keeps identical tuple bytes after; bitmap `before & !after == 0`; status pages change only from InProgress; counters only rise.
- The sweep never hits "file past span, count 1", 0 of 48 images: `cross_a_group` checkpoints before its first flush. Add `flush_pages` before that checkpoint.
- A refused insert leaves its row and earlier index entries as dead data: storage.md's "before anything is written" holds for the allocator batch, not the call. A full page that compacts was not exercised.
- A verify of a zeroed page 8 (kind 7, owner 2) reports one finding and no mark.
