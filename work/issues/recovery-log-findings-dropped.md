---
created-at: 2026-11-05
status: resolved
kind: bug
priority: 345
summary: "recovery's log findings (a stale block past the end) are kept in Recovery.log, discarded by the caller, then erased by clear_tail"
closed: 2026-11-06
tags:
  - class-damage-unreported
---

# Recovery's log-tail findings are never reported

## Report

## 2026-11-05

From the damage-reporting audit, confirmed in code: storage/recovery.rs builds `Recovery { log: s.findings, .. }` after `clear_tail` has zeroed or removed the blocks, and api/mod.rs and create.rs discard the `Recovery` (`Ok((p, _))`). A valid block with a sequence at or below the last accepted, past the end of the log, is `LogProblem::Stale` (block.rs:417-421); storage.md §7's table says it is reported as log damage. Nothing reports it, and the evidence is erased before any verify. Direction: record the findings durably (a mark or a verify-visible log finding) before `clear_tail`. Defect class: Damage reported as nothing.

## 2026-11-06: fixed by the damage log

Fixed in `feat: record every detected page damage durably` (damage-marked-at-detection change 1). Recovery writes stale log blocks past the end to the file header and forces them before `clear_tail`; the closing checkpoint folds them into the damage list, and verify reports them as log findings. Past 62 entries an overflow bit records the whole file and verify reports `Unlisted`. Tests in crates/adtb-sim/tests/sim/damage_log.rs: `stale_log_blocks_past_the_end_are_reported_after_a_crash_anywhere_in_the_open` and `one_stale_log_block_past_what_the_header_holds_records_the_whole_file`.

## 2026-11-06: resolved
