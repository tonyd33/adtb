---
created-at: 2026-11-06
status: resolved
kind: bug
priority: 260
summary: "Pager::damage evicts a dirty frame and drops it from the dirty table, so the checkpoint passes its logged change and the change is lost"
closed: 2026-11-06
tags:
  - class-damage-unreported
---

# Damaging a dirty page loses its logged change

## Report

## 2026-11-06

Found by the read-in gate (readin-gate-2 track) through clog_lost::the_first_verify_through_a_server_of_a_status_page_that_lost_a_write_reports_it: after verify reports status page 12 stale, `Pager::damage(12)` evicts its dirty frame holding a later logged change (the server session statuses) and drops it from the dirty table; the close checkpoint moves past that record, so the file keeps the older page and the change is gone. On main this loses committed statuses silently. Fix in the readin-gate-2 track: damage keeps the page in the dirty table at its oldest unflushed LSN without writing it, so redo replays the change; every other dirty-frame drop path checked.

## 2026-11-06: resolved

Landed in `fix: check every page read in against its expected LSN`. `Pager::damage` keeps the page's dirty-table entry, so the checkpoint's redo never passes a logged change nothing wrote; the next open replays it. Test recycle::a_change_to_a_page_damaged_before_its_write_is_replayed_by_the_next_open fails with the old `m.dirty.remove`, and the open_damage dropping tests, the real-file variant included, now assert the finding they mean and fail with it too. The cost, the entry pinning the redo point for the rest of the open, is stated in storage.md §10.3 and filed as damaged-dirty-entry-outlives-free.

## 2026-11-06: resolved
