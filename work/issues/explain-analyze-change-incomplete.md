---
created-at: 2026-10-26
status: open
kind: bug
priority: 693
summary: explain analyze leaves temp relation effects out of the change and counts one actual row for a write
tags:
  - quality
  - engine
---

# explain analyze of a write to a temp relation reports an empty change and one actual row

## Report

`run_here` (`crates/adtb/src/api/command.rs:199-225`) takes the temp relation effects out before `boundary`, so `Outcome.inserted` and `Outcome.deleted` never include them, and `explain analyze` reports `change: {}` for an insert into a temp relation. Separately, `rows` (`crates/adtb/src/write/eval.rs:240-246`) counts 1 for any result that is not a set or list, so every write shows `actual rows 1`.

```adt
temp relation t : { a: I64 } key a
explain analyze insert t { a: 1 }
```

Actual: `change: {}` and `actual rows 1`.

Expected: the change names one row inserted into `t`, and a write's result has no row count (or 0).

Fix: carry temp relation effects in the outcome, or count them in the analysed change, and leave `rows` out for a change or unit result.
