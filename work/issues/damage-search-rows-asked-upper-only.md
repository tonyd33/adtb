---
created-at: 2026-11-10
status: open
kind: bug
priority: 350
summary: "the damage search counts a stated row as asked when only its upper bound was judged, so an engine that never sets the row's mark passes the rows-asked floor"
tags:
  - quality
  - test
  - sim
  - class-weak-tests
---

# Bug: a stated row counts as asked with only its upper bound judged

## Report

`judge` (crates/adtb-sim/tests/storage/damage_search/judge.rs:367-378) adds `p.row` to the rows asked when `p.stale && (p.must_report || p.open || named)`. The mark is required present only for `must_report` pages (`expected`, judge.rs:340-344). A stale page verify names that is not `must_report` (every `LostEarlier` page, every freed `LostCurrent` page) gets only the check that no mark exceeds what is allowed, yet `assert_rows_asked` (mod.rs:503) counts its row, such as `(Damaged, Relation, Opening)`. An engine that never marks such a page passes the floor.

Fix direction: count a row as asked only when its lower bound was judged, or keep the rows bounded above only apart and floor them separately.

Found in the quality review of `reviewed..main` at 7bf40103.
