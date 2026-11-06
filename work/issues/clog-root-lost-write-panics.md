---
created-at: 2026-11-05
status: resolved
kind: bug
priority: 300
summary: "a lost write of the commit-log root panics the background thread after open: clog.rs:93 status 1025 resolved twice, from Committed to Committed"
closed: 2026-11-06
tags:
  - class-damage-unreported
---

# A lost write of the commit-log root panics after open

## Report

## 2026-11-05

Found by the verify-fatal author (slot-44, search test over every page an insert changes). Reverting the commit-log root page to its copy before an insert, then reopening, panics a background thread at crates/adtb/src/mvcc/clog.rs:93 with `status 1025 resolved twice, from Committed to Committed`; no verify is involved. A panic in place of a finding or a quarantine. The verify-fatal search test excludes the root by name for this; remove the exclusion with the fix. Reproduce first, then state what the root holds that settle or reserve trusts. Defect class: Damage reported as nothing (here, as a panic).

## 2026-11-06: fixed by the read-in gate

Fixed in 0d1250ce (damage-readin-gate). The commit-log root's read-in is refused Stale at open instead of reaching settle. `a_lost_write_of_each_page_an_insert_changes_is_refused_on_read_and_reported_by_verify` in crates/adtb-sim/tests/sim/clog_lost.rs reverts every page an insert changes, the root included with its exclusion removed, and each is refused on its next read.

## 2026-11-06: resolved
