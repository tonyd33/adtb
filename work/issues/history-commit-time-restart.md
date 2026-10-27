---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 799
summary: "commit times are kept non-decreasing only within one process, so after a restart an at instant read can pick the wrong marker"
closed: 2026-10-27
tags:
  - engine
---

# Bug: commit time order restarts at reopen

## Report

With history on, `Transactions::commit` keeps commit times non-decreasing, but the floor starts from 0 at each open. A clock that steps back across a restart gives a later commit an earlier time than one already recorded, and `at instant` then resolves to the wrong marker. Seed the floor from the latest time in the retention record at open. Also: each commit does a B-link insert into the record under the transactions state lock.

## 2026-10-27: done

`Transactions::keep_history(pager, past)` seeds the commit time floor from `Past::latest_commit`, which scans the record's commit entries once at open. Test `a_clock_that_steps_back_across_a_reopen_keeps_the_order_of_commit_times` in `crates/adtb-sim/tests/history.rs` fails without the fix.

Open: the B-link insert per commit under the transactions state lock. Nothing in the engine calls `keep_history` yet outside the sim tests.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qexec track; what stays open is in `exec-scan-follow-ups`.
