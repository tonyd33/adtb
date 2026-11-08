---
created-at: 2026-11-06
status: resolved
kind: bug
priority: 480
summary: "a damaged dirty page's dirty-table entry outlives a free of the page and pins the redo point for the open"
closed: 2026-11-08
tags:
  - class-none
  - engine
---

# Damaged dirty entry outlives a free

## Report

## Report

Since the read-in gate's change 2, `Pager::damage` of a dirty frame keeps the page's dirty-table entry so the checkpoint never passes a logged change nothing wrote. The entry stays for the rest of the open: it pins the checkpoint's redo point, so recycling stops and the log grows, and it outlives a later `free` of the page. Found by the change 2 review (N4). Decide when the entry may go: after the page is freed and the free is durable, or after a repair rewrites the page.

## 2026-11-08: resolved

Fixed in storage owners step 8. Cause: one table state, an entry with no unwritten changes, meant both "written, waiting for a force" and "copy dropped unwritten", so a force that covered an earlier write of a page changed since and then refused with the change unwritten dropped its entry, and the next checkpoint passed the logged change (shown on main: a crash after that checkpoint reopens with the page refused `Stale`). A page the force did not cover kept its entry for the rest of the open, past any free. `DirtyTable` now has a third frame state, `Lost`, set by `refused` when the copy held changes the file lacks: a force never drops or raises it, `freeing` notes the freed record logged of it, and `freed_durable`, called from the log force once that record is durable, drops it; a later chain start makes it an ordinary dirty entry, so a durable free of an earlier incarnation leaves it. A copy written before its refusal leaves at the next covering force, as before. Tests: pagetable.rs unit tests per transition and the model sweep; recycle_tests::a_change_dropped_unwritten_with_its_page_is_replayed_until_the_page_is_durably_freed, over crash points. Reach: on main a durable committed change was lost on close under a whole quarantine: a page written, changed and refused through `Pager::damage` lost its entry to the close's `flush_pages`, its `checkpoint` passed the change, and the next open refused the page `Stale` (recycle_tests::a_change_refused_unwritten_under_a_whole_quarantine_reads_back_after_the_close, fails on main). A copy is dropped unwritten only under a whole quarantine, where no free is logged, so `freed_durable` drops an entry only after `release(Database)`, which is testing only. storage.md §6 and §10.3 state the third way an entry leaves.
