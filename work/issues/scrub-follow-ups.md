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
