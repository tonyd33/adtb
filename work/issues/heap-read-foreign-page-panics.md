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

## 2026-11-02: run, worse than the report

Run through `Database` on the rot fixture: a heap's first page overwritten by another heap's page with a valid checksum. No panic and no refusal: scans and index lookups crossing it answered fewer rows than acknowledged (rows 0 of 5, docs.Small 0 of 40), and an update and a delete by key on it committed and changed nothing. Cause: `Heap::scan_pages` skips a foreign page and `heap::walk` answers no row for it, both on purpose for a page vacuum freed and reused under the reader. So the panics are not reached first; a wrong answer is. Class: identity by position, a page id standing for a page of this heap.

Ruling: a foreign or empty start page whose naming entry is still present after the read is damage, and refuses and marks its holder. For a scan the directory still names the page; for a lookup or write target the index still holds the entry. A same-owner page lacking the slot is caught by the lookup check; a scan cannot tell it from a lost write, which verify covers.

## 2026-11-02: fix in review, workspace foreign-page

Change qrrtmwrn in ../adtb-slot-41, receipt work/briefs/receipt-foreign-page-r1.md (local). A page of the heap's own owner whose slot is unused while an index entry names it refuses naming the index and marks only the index; verify's Orphan test unchanged. Review found no blocker; one delta asked: a typed index refusal in place of `PageError { pid: root, damage: Checksum }`, the large store owner check restored with a case reaching it, a healthy stale-entry case after vacuum. Then land.

Left after the fix, severe first, from the review:

- A chain whose successor lies on a foreign page ends the row silently: `walk` returns `None` and `Store::fetch` answers no row. Telling it from vacuum freeing the successor needs a reread of the predecessor's `next`.
- ABA window in `refuse_named` and `refuse_if_named`: a slot or page freed, reused and freed again between the two reads refuses a healthy database. Needs a write and vacuum inside a few page reads; no test.
- `pager.damage` on a readable foreign page drops its dirty state; with both owners naming the page the other owner's unflushed changes leave memory. Whether redo restores them is unchecked.
- `history::probe` still skips a foreign page silently.
- A scan of an emptied page of the heap's own owner drops rows silently; after the index is quarantined, reads fall back to that scan, so an acknowledged row answered empty in the test. Lost write, verify's domain by the ruling above.
- Cost: `Store::fetch` reads one more page per entry with no visible version; `refuse_if_named` walks the whole directory per skipped page.
- `fetch_past`, the large store's `chunks`, retain, release, recount and freeing are untested.

## 2026-11-02: landed

`fix: refuse a heap page another owner holds`. The delta landed as asked: `heap::OrphanEntry { index }` with `exec::Error::OrphanEntry` and `write::Error::OrphanEntry`, which `quarantining` maps to `Quarantine::Index`; the large store's `tuple_at` checks kind, owner and chunk length for every chunk read, and `rot_cross` requires the replaced page in the pager's damaged set; `lost_bitmap::lookups_racing_a_vacuum_that_frees_slots_of_a_page_it_keeps_are_not_refused` searches 8 seeds of a key and tag lookup racing insert, delete and vacuum on a page that keeps other rows. Check exit 0, 3566 passed. Production +259 -95 against a 120 line stop line, accepted across review rounds.

The panics this ticket reported are gone; it stays open for the leftovers above, and two more: the apply unique check's recheck survives its mutant, the race inserting only fresh keys, and `fetch_past` has no case, the fixture keeping no history.

Direction: return `pager.damage(pid, Damage::Checksum)` as `read_role` does (crates/adtb/src/mvcc/clog.rs), sweeping every assert and panic in heap/handle.rs and heap/large.rs that judges page contents. space-append-spin covers the directory walk and stays separate.
