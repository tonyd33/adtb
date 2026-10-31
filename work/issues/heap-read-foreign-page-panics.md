---
created-at: 2026-10-31
status: open
kind: bug
priority: 478
summary: a heap or large-value read of a formatted page another owner holds panics the process instead of refusing the page
tags:
  - quality
  - engine
---

# Heap read foreign page panics

## Report

The lost-bitmap fix turned the commit log's and the sequences' page checks into refusals, and `read_formatted` refuses a blank page. A page that is formatted but held by another owner still panics on the read path, and a panic crashes the process:

- crates/adtb/src/heap/handle.rs:398 `assert_eq!(self.id, storage::owner(&page), ...)` in `Heap::read`, and the same assert at handle.rs:318;
- crates/adtb/src/heap/handle.rs:737 `panic!("{at:?} holds no version")`;
- crates/adtb/src/heap/large.rs:79, :249 and :274, a missing chunk of a large value.

Damage reaches these. The space-append-spin review shows the route: an intact but wrong bitmap, as after a lost write, lets a page be reused while an entry still names it, and the reader then finds a valid checksum and a foreign owner. One bad page ends the process where the storage contract says it quarantines its holder (docs/design/storage.md section 3).

Found by reading; not run. Repro: in crates/adtb-sim/tests/lost_bitmap.rs's setup, `pager.free` a heap page a row lives on, insert into another relation until that page is reused, then read the row by key.

Actual: the process panics with `is not in heap`. Expected: the call answers `Quarantined` and the page is refused.

Direction: return `pager.damage(pid, Damage::Checksum)` as `read_role` does (crates/adtb/src/mvcc/clog.rs), sweeping every assert and panic in heap/handle.rs and heap/large.rs that judges page contents. space-append-spin covers the directory walk and stays separate.
