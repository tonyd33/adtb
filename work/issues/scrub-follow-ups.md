---
created-at: 2026-10-30
status: open
kind: bug
priority: 470
summary: "scrub: an allocated page nothing reaches marks the database; a due open reads the whole retained log; close writes the cursor; a transient read error refuses a page; a busy daemon with main parked hangs the simulator; a torn read has no regression test"
---

# Scrub follow ups

## Report

Also: the mark is durable before its finding row, so a crash between them leaves a relation quarantined with no row saying why; a panic in the log read at the start of a pass is not caught; a free-page bitmap that is unreadable or of the wrong kind.

## 2026-11-03: from the scrub search

Track scrub-search (crates/adtb-sim/tests/maintenance_faults.rs) searches rot through the scrub started by `ScrubInterval`. It reproduced "a transient read error refuses a page": three failed reads in `Pager::check_page` recorded `Unreadable` in the pager's damaged map, so every later read failed `Fatal { Storage }`. A first fix filed it as `Checksum` and was withdrawn: storage.md says `Unreadable` marks nothing and stays a storage failure. The fix in review keeps `Unreadable` and stops recording it, so the next read retries.

Left from the review:
- `names_page` in the scrub test counts any held finding as naming an unowned page, so an ownerless page under a database-wide mark passes unnamed.
- The crash test asserts `held_findings() == 0` after a restart behind a whole-database mark, pinning the "mark durable before its finding row" gap as expected behaviour.
- A database marked whole at open runs no scrub reads; a page the pager holds damaged is not reread.

## 2026-11-03: landed

`fix: leave a page the scrub cannot read for the next read to retry`. `check_page` keeps `Unreadable` as an I/O failure, marks nothing and does not record it, so "a transient read error refuses a page" is closed for the scrub and verify. The scrub search landed with it: rot found by a pass alone and filed as a read would, a crash during a pass, and 1, 2, 3 failed reads plus a bad first read then two failed rechecks. Check exit 0, 3609 passed.

Left: the read path in pager.rs (about line 661) still records one failed `Unreadable` read by a call as sticky, refusing the page until reopen; the storage.md rule says an I/O failure marks nothing, so a call's single failed read should not refuse the page for every later reader either. Nothing finds a rotted copy of the file header: the scrub starts at the first data page and open uses the other copy.
