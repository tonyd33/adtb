---
created-at: 2026-10-27
status: open
kind: bug
priority: 799
summary: "commit times are kept non-decreasing only within one process, so after a restart an at instant read can pick the wrong marker"
tags:
  - engine
---

# Bug: commit time order restarts at reopen

## Report

With history on, `Transactions::commit` keeps commit times non-decreasing, but the floor starts from 0 at each open. A clock that steps back across a restart gives a later commit an earlier time than one already recorded, and `at instant` then resolves to the wrong marker. Seed the floor from the latest time in the retention record at open. Also: each commit does a B-link insert into the record under the transactions state lock.
