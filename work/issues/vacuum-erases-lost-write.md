---
created-at: 2026-11-05
status: open
kind: bug
priority: 350
summary: "background vacuum prunes a heap page whose write was lost, so verify reports Orphan index findings instead of StalePage"
---

# Vacuum erases the evidence of a lost heap write

## Report

## 2026-11-05

Found by the clog-lost review. After a heap page's write is lost, the background vacuum can write to that page before `verify` reads it. Verify then reports two `Orphan` index findings ("page N slot M: holds no version of the entry's key") and no `StalePage`. The lost write is reported as index damage on the wrong object, and the heap that lost rows is never named.

Reproduction: in crates/adtb-sim/tests/sim/verify.rs, `a_lost_write_is_a_stale_page_finding`, set `no_background_vacuum` back to false. Since clog-lost, the extra log read at open shifts the schedule, so the worker runs before verify.

Direction: vacuum must not write a page whose LSN is behind what the retained log says it must be (check `lost_write` before vacuum prunes a page), or verify must still name the page. Then remove the flag from the test.

Defect class: Damage reported as nothing (here, reported as different damage).
