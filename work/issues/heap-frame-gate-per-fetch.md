---
created-at: 2026-11-17
status: open
kind: bug
priority: 300
summary: "the heap page frame check runs on every Heap::read, so a row fetched through an index costs about 70% more than before the check"
tags:
  - engine
  - perf
  - class-none
---

# Heap frame gate per fetch

## Report

Measured by page-decode part 2 (receipt-page-decode-r2, round 3), release, a 4 KiB page of 101 slots: `HeapPage::any` 7 to 177 ns, `Heap::read` about 245 to 418 ns. `slotted::framed` checks every line pointer each time a page becomes a `HeapPage`, and `Heap::read` (the index fetch path, crates/adtb/src/exec/store.rs) does that once per row. Scan pays it once per page.

Direction: check a page's frame once per incarnation in memory, when its bytes enter the buffer from disk, and keep the verdict with the frame; pages the engine writes in memory stay framed by the writers. Keep one gate. Measure an index fetch end to end before and after.
