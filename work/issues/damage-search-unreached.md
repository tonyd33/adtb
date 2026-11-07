---
created-at: 2026-11-07
status: open
kind: bug
priority: 260
summary: "the damage search leaves 15 cells unreached (freed commit log and engine pages, a freed index read) and does not judge backup copies' refusals, a crash inside a migration's open or the scrub task"
tags:
  - class-weak-tests
---

# Cells and consumers the damage search does not reach

## Report

## 2026-11-07

The damage class search (damage_search in crates/adtb-sim/tests/storage) lists cells no sequence reaches in `UNREACHED`, and fails when one becomes reached:

- LostEarlier CommitLog: an 8 KiB status page holds about 32K ids and a sequence assigns a few thousand, so truncation frees none. A smaller page size for a commit-log family of seeds may reach it.
- LostEarlier Engine: no sequence frees an engine-relation page.
- LostEarlier Index x Read: only tags' index pages are freed, and the drop that frees them leaves no read of tags.

Not judged, from the round 1 review: `judge_copy` ignores the refusals of backup copies; versions and started for a crash inside a migration's open; the scrub task in api/maintenance.rs (the search calls `scrub_page` directly).

Direction: reach each listed cell or state the rule that makes it unreachable, and judge the three consumers.

Defect class: Tests asserting less than they claim.
